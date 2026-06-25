// 文件变更审查：基于 git 快照
//
// 设计：
// - 每轮 AI 回合开始时调用 create_turn_snapshot 在工作区当前状态拍一个隐藏 ref 快照。
// - 回合正常结束时再拍一个结束快照，用 git diff 计算本轮改动，写一条 batch 进 SQLite。
// - file_change_batches 表新增 snapshot_sha（起点）/end_snapshot_sha（结束）两列。
// - 审查页 diff 不再存 before/after 正文，统一从 git 快照 diff 得出。
// - 回退 batch = git read-tree + clean 到起点快照，工作区回到本轮开始那一刻。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::{Row, SqlitePool};
use tauri::AppHandle;

pub use crate::llm::services::git_snapshot::{
    FileChangeBatch, FileChangeBatchSummary, FileChangeEntry, FileDiffLine,
};

/// Read file as UTF-8 string, stripping BOM (\u{FEFF}) if present.
///
/// PowerShell `Get-Content` strips BOM by default, but `std::fs::read_to_string`
/// preserves it. Using this helper ensures the AI and file editing tools see the same
/// content, preventing context matching failures on BOM-prefixed files.
pub(crate) fn read_file_utf8(path: &std::path::Path) -> Result<String, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Error reading {}: {}", path.display(), e))?;
    Ok(content
        .strip_prefix('\u{FEFF}')
        .unwrap_or(&content)
        .to_string())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_conversation_id(conversation_id: Option<&str>) -> Result<String, String> {
    conversation_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "conversation_id is required for file change review".to_string())
}

pub fn resolve_tool_path(raw_path: &str) -> Result<std::path::PathBuf, String> {
    crate::llm::utils::paths::resolve_absolute_path_for_write(raw_path, "path")
}

/// Write/Edit 工具专用：堆文件并返回受影响的 *display* 路径。
/// 不再写 SQLite、不再有写文件前/失败回滚——回退完全交给 git 快照。
pub fn write_file_simple(
    target: &Path,
    content: &str,
) -> Result<String, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", target.display(), e))?;
    }
    std::fs::write(target, content)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
    Ok(target.display().to_string())
}

// 回合开始：在工作区拍一个起点快照。失败（无 git / 无改动）时静默忽略，不阻断回合。
pub async fn create_turn_snapshot(
    app: &AppHandle,
    conversation_id: Option<&str>,
) {
    let Ok(conv_id) = normalize_conversation_id(conversation_id) else {
        return;
    };
    let Ok(repo_root) =
        crate::command::workspace::workspace_root_for_conversation(app, Some(&conv_id))
    else {
        return;
    };
    if crate::llm::services::git_snapshot::ensure_repo(&repo_root).is_err() {
        return;
    }
    let ref_name = format!("turn-start-{conv_id}-{}", now_millis());
    if let Ok(Some(sha)) =
        crate::llm::services::git_snapshot::create_snapshot(&repo_root, &ref_name)
    {
        persist_pending_snapshot(app, &conv_id, &sha).await.ok();
    }
}

async fn persist_pending_snapshot(
    app: &AppHandle,
    conversation_id: &str,
    sha: &str,
) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    sqlx::query(
        r#"
        INSERT INTO conversation_change_snapshots (id, conversation_id, snapshot_sha, created_at)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(conversation_id)
    .bind(sha)
    .bind(now_millis() as i64)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 拉最近一个未消费的起点快照 SHA（FIFO），标记 used 后返回 SHA 供回合收尾 diff 用。
async fn take_pending_snapshot_sha(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let row = sqlx::query(
        r#"
        SELECT id, snapshot_sha FROM conversation_change_snapshots
        WHERE conversation_id = ? AND consumed_at IS NULL
        ORDER BY created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .bind(conversation_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id: String = row.get("id");
    let sha: String = row.get("snapshot_sha");
    sqlx::query(
        "UPDATE conversation_change_snapshots SET consumed_at = ? WHERE id = ?",
    )
    .bind(now_millis() as i64)
    .bind(&id)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(Some(sha))
}

/// 回合收尾：拍结束快照，对比起点快照，写一条 batch。
/// 仅在确有改动时返回 batch_id。
pub async fn record_turn_changes(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Option<String> {
    let conv_id = normalize_conversation_id(conversation_id).ok()?;
    let repo_root = crate::command::workspace::workspace_root_for_conversation(app, Some(&conv_id)).ok()?;
    if crate::llm::services::git_snapshot::ensure_repo(&repo_root).is_err() {
        return None;
    }

    let start_sha = take_pending_snapshot_sha(app, &conv_id).await.ok().flatten()?;

    let end_ref = format!("turn-end-{conv_id}-{}", now_millis());
    let end_sha = crate::llm::services::git_snapshot::create_snapshot(&repo_root, &end_ref)
        .ok()?
        .unwrap_or_default();

    let (files, additions, deletions) =
        match crate::llm::services::git_snapshot::diff_snapshots(
            &repo_root,
            Some(&start_sha),
            if end_sha.is_empty() { None } else { Some(&end_sha) },
        ) {
            Ok(v) if !v.0.is_empty() => v,
            Ok(_) => return None,
            Err(_) => return None,
        };

    let batch_id = uuid::Uuid::new_v4().to_string();
    if let Err(err) = persist_batch(
        app,
        &conv_id,
        &batch_id,
        &start_sha,
        if end_sha.is_empty() { None } else { Some(&end_sha) },
        &files,
        additions,
        deletions,
    )
    .await
    {
        eprintln!("[file_changes] record_turn_changes 写库失败: {err}");
        return None;
    }
    Some(batch_id)
}

/// 仅消费掉当前待用起点快照，不生成 batch（用于回合异常终止）。
pub async fn discard_pending_snapshot(app: &AppHandle, conversation_id: Option<&str>) {
    let Ok(conv_id) = normalize_conversation_id(conversation_id) else {
        return;
    };
    let _ = take_pending_snapshot_sha(app, &conv_id).await;
}

async fn persist_batch(
    app: &AppHandle,
    conversation_id: &str,
    batch_id: &str,
    snapshot_sha: &str,
    end_snapshot_sha: Option<&str>,
    files: &[FileChangeEntry],
    additions: usize,
    deletions: usize,
) -> Result<(), String> {
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        r#"
        INSERT INTO file_change_batches (
            id, conversation_id, tool_name, created_at,
            reverted, reverted_at,
            snapshot_sha, end_snapshot_sha
        ) VALUES (?, ?, ?, ?, 0, NULL, ?, ?)
        "#,
    )
    .bind(batch_id)
    .bind(conversation_id)
    .bind("AI 回合")
    .bind(now_millis() as i64)
    .bind(snapshot_sha)
    .bind(end_snapshot_sha)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    for (index, file) in files.iter().enumerate() {
        let diff_json = serde_json::to_string(&file.diff)
            .map_err(|e| format!("序列化 diff 失败: {}", e))?;
        sqlx::query(
            r#"
            INSERT INTO file_change_files (
                batch_id, file_order, path, absolute_path,
                change_type, before_text, after_text, diff_json,
                additions, deletions
            ) VALUES (?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?)
            "#,
        )
        .bind(batch_id)
        .bind(index as i64)
        .bind(&file.path)
        .bind(&file.absolute_path)
        .bind(&file.change_type)
        .bind(diff_json)
        .bind(additions as i64)
        .bind(deletions as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list_change_batches(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<Vec<FileChangeBatchSummary>, String> {
    let Some(conversation_id) = conversation_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
    else {
        return Ok(Vec::new());
    };
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let rows = sqlx::query(
        r#"
        SELECT
            b.id, b.conversation_id, b.tool_name, b.created_at,
            b.reverted, b.reverted_at,
            COUNT(f.id) AS file_count,
            COALESCE(SUM(f.additions), 0) AS additions,
            COALESCE(SUM(f.deletions), 0) AS deletions
        FROM file_change_batches b
        LEFT JOIN file_change_files f ON f.batch_id = b.id
        WHERE b.conversation_id = ?
        GROUP BY b.id
        ORDER BY b.created_at DESC, b.id DESC
        "#,
    )
    .bind(&conversation_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let id_for_paths = id.clone();
        summaries.push(FileChangeBatchSummary {
            id,
            conversation_id: row.get("conversation_id"),
            tool_name: row.get("tool_name"),
            created_at: row.get::<i64, _>("created_at").max(0) as u64,
            reverted: row.get::<i64, _>("reverted") != 0,
            reverted_at: row
                .get::<Option<i64>, _>("reverted_at")
                .map(|v| v.max(0) as u64),
            file_count: row.get::<i64, _>("file_count").max(0) as usize,
            additions: row.get::<i64, _>("additions").max(0) as usize,
            deletions: row.get::<i64, _>("deletions").max(0) as usize,
            paths: load_batch_paths(&pool, &id_for_paths).await.unwrap_or_default(),
        });
    }
    Ok(summaries)
}

async fn load_batch_paths(pool: &SqlitePool, batch_id: &str) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>(
        "SELECT path FROM file_change_files WHERE batch_id = ? ORDER BY file_order ASC",
    )
    .bind(batch_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

pub async fn get_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    batch_id: &str,
) -> Result<FileChangeBatch, String> {
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let conversation_id = normalize_conversation_id(conversation_id)?;
    let batch_row = sqlx::query(
        r#"
        SELECT id, conversation_id, tool_name, created_at, reverted, reverted_at, snapshot_sha, end_snapshot_sha
        FROM file_change_batches
        WHERE conversation_id = ? AND id = ?
        "#,
    )
    .bind(&conversation_id)
    .bind(batch_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "找不到审查记录".to_string())?;

    let snapshot_sha: Option<String> = batch_row.get("snapshot_sha");
    let end_snapshot_sha: Option<String> = batch_row.get("end_snapshot_sha");

    // 优先：直接从 file_change_files 读已存的 diff。这样即便工作区已变化仍能查看。
    let file_rows = sqlx::query(
        r#"
        SELECT path, absolute_path, change_type, diff_json
        FROM file_change_files
        WHERE batch_id = ?
        ORDER BY file_order ASC
        "#,
    )
    .bind(batch_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut files = Vec::with_capacity(file_rows.len());
    for row in file_rows {
        let diff_json: String = row.get("diff_json");
        let diff = serde_json::from_str::<Vec<FileDiffLine>>(&diff_json)
            .map_err(|e| format!("解析 diff 失败: {}", e))?;
        files.push(FileChangeEntry {
            path: row.get("path"),
            absolute_path: row.get("absolute_path"),
            change_type: row.get("change_type"),
            before: None,
            after: None,
            diff,
        });
    }

    // 退化路径：库里没存 diff_json（理论不会发生），则用 git 重新 diff 两个快照。
    if files.is_empty() {
        if let (Some(start), Some(end)) = (snapshot_sha.as_deref(), end_snapshot_sha.as_deref()) {
            let root = crate::command::workspace::workspace_root_for_conversation(
                app,
                Some(&conversation_id),
            )?;
            if let Ok((repo_files, _, _)) =
                crate::llm::services::git_snapshot::diff_snapshots(&root, Some(start), Some(end))
            {
                files = repo_files;
            }
        }
    }

    Ok(FileChangeBatch {
        id: batch_row.get("id"),
        conversation_id: batch_row.get("conversation_id"),
        tool_name: batch_row.get("tool_name"),
        created_at: batch_row.get::<i64, _>("created_at").max(0) as u64,
        reverted: batch_row.get::<i64, _>("reverted") != 0,
        reverted_at: batch_row
            .get::<Option<i64>, _>("reverted_at")
            .map(|v| v.max(0) as u64),
        files,
    })
}

/// 回退一次 AI 回合：git read-tree + clean 把工作区拉回起点快照那一刻。
pub async fn revert_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    batch_id: &str,
) -> Result<FileChangeBatch, String> {
    let conversation_id = normalize_conversation_id(conversation_id)?;
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let row = sqlx::query(
        r#"SELECT id, conversation_id, tool_name, created_at, reverted, reverted_at, snapshot_sha
           FROM file_change_batches WHERE conversation_id = ? AND id = ?"#,
    )
    .bind(&conversation_id)
    .bind(batch_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "找不到审查记录".to_string())?;

    if row.get::<i64, _>("reverted") != 0 {
        return Err("这次变更已经回退过了".to_string());
    }
    let snapshot_sha: Option<String> = row.get("snapshot_sha");

    if let Some(sha) = snapshot_sha.as_deref() {
        let root =
            crate::command::workspace::workspace_root_for_conversation(app, Some(&conversation_id))?;
        crate::llm::services::git_snapshot::revert_to_snapshot(&root, sha)?;
    } else {
        return Err("此 batch 未关联 git 快照，无法回退".to_string());
    }

    let now = now_millis() as i64;
    sqlx::query(
        "UPDATE file_change_batches SET reverted = 1, reverted_at = ? WHERE conversation_id = ? AND id = ?",
    )
    .bind(now)
    .bind(&conversation_id)
    .bind(batch_id)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    get_change_batch(app, Some(&conversation_id), batch_id).await
}

/// 会话删除时连带清理。
pub async fn delete_changes_for_conversation(app: &AppHandle, conversation_id: &str) {
    let pool = match crate::llm::history::get_pool_with_schema(app).await {
        Ok(p) => p,
        Err(_) => return,
    };
    let _ = sqlx::query(
        "DELETE FROM file_change_files WHERE batch_id IN (SELECT id FROM file_change_batches WHERE conversation_id = ?)",
    )
    .bind(conversation_id)
    .execute(&pool)
    .await;
    let _ = sqlx::query("DELETE FROM file_change_batches WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM conversation_change_snapshots WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await;
}

/// 全局清空（hard reset 应用数据时用）。
pub async fn delete_all_changes(app: &AppHandle) {
    let pool = match crate::llm::history::get_pool_with_schema(app).await {
        Ok(p) => p,
        Err(_) => return,
    };
    let _ = sqlx::query("DELETE FROM file_change_files").execute(&pool).await;
    let _ = sqlx::query("DELETE FROM file_change_batches").execute(&pool).await;
    let _ = sqlx::query("DELETE FROM conversation_change_snapshots")
        .execute(&pool)
        .await;
}