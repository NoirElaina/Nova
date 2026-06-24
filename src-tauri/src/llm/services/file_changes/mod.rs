use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use sqlx::{Row, SqlitePool};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tokio::sync::Mutex;

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

#[derive(Debug, Clone)]
pub struct FileEditResult {
    pub files: Vec<String>,
    pub change_batch_id: Option<String>,
}

pub(crate) async fn commit_drafts(
    app: &AppHandle,
    conversation_id: Option<&str>,
    tool_name: &str,
    drafts: Vec<FileChangeDraft>,
) -> Result<FileEditResult, String> {
    let files = drafts
        .iter()
        .filter(|draft| draft.before != draft.after)
        .map(|draft| path_for_display(&draft.path))
        .collect::<Vec<_>>();
    let change_batch_id = commit_change_batch(app, conversation_id, tool_name, drafts).await?;
    let files = if change_batch_id.is_some() {
        files
    } else {
        Vec::new()
    };

    Ok(FileEditResult {
        files,
        change_batch_id,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct FileChangeDraft {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffLine {
    pub kind: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeEntry {
    pub path: String,
    pub absolute_path: String,
    pub change_type: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub diff: Vec<FileDiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeBatch {
    pub id: String,
    pub conversation_id: String,
    pub tool_name: String,
    pub created_at: u64,
    pub reverted: bool,
    pub reverted_at: Option<u64>,
    pub files: Vec<FileChangeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeBatchSummary {
    pub id: String,
    pub conversation_id: String,
    pub tool_name: String,
    pub created_at: u64,
    pub reverted: bool,
    pub reverted_at: Option<u64>,
    pub file_count: usize,
    pub additions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
}

static FILE_CHANGE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const MAX_STORED_BATCHES: usize = 200;

fn file_change_lock() -> &'static Mutex<()> {
    FILE_CHANGE_LOCK.get_or_init(|| Mutex::new(()))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_conversation_id(conversation_id: Option<&str>) -> Result<String, String> {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "conversation_id is required for file change review".to_string())
}

pub fn resolve_tool_path(raw_path: &str) -> Result<PathBuf, String> {
    crate::llm::utils::paths::resolve_absolute_path_for_write(raw_path, "path")
}

fn path_for_display(path: &PathBuf) -> String {
    path.display().to_string()
}

fn change_type(before: &Option<String>, after: &Option<String>) -> String {
    match (before, after) {
        (None, Some(_)) => "added",
        (Some(_), None) => "deleted",
        _ => "modified",
    }
    .to_string()
}

fn build_entry(draft: FileChangeDraft) -> FileChangeEntry {
    let diff = diff_lines(draft.before.as_deref(), draft.after.as_deref());
    FileChangeEntry {
        path: path_for_display(&draft.path),
        absolute_path: draft.path.display().to_string(),
        change_type: change_type(&draft.before, &draft.after),
        before: draft.before,
        after: draft.after,
        diff,
    }
}

fn build_batch(
    conversation_id: &str,
    tool_name: &str,
    drafts: Vec<FileChangeDraft>,
) -> Option<FileChangeBatch> {
    let mut seen = BTreeSet::new();
    let files = drafts
        .into_iter()
        .filter(|draft| draft.before != draft.after)
        .filter(|draft| seen.insert(draft.path.clone()))
        .map(build_entry)
        .collect::<Vec<_>>();

    if files.is_empty() {
        return None;
    }

    Some(FileChangeBatch {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        tool_name: tool_name.to_string(),
        created_at: now_millis(),
        reverted: false,
        reverted_at: None,
        files,
    })
}

pub(crate) async fn commit_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    tool_name: &str,
    drafts: Vec<FileChangeDraft>,
) -> Result<Option<String>, String> {
    let conversation_id = normalize_conversation_id(conversation_id)?;
    let Some(batch) = build_batch(&conversation_id, tool_name, drafts) else {
        return Ok(None);
    };
    let batch_id = batch.id.clone();
    let _guard = file_change_lock().lock().await;
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    validate_batch_current_state(&batch)?;
    apply_batch_after(&batch)?;
    if let Err(error) = persist_batch(&pool, &batch).await {
        rollback_batch_after(&batch)?;
        return Err(error);
    }
    Ok(Some(batch_id))
}

async fn persist_batch(pool: &SqlitePool, batch: &FileChangeBatch) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    sqlx::query(
        r#"
        INSERT INTO file_change_batches (
            id,
            conversation_id,
            tool_name,
            created_at,
            reverted,
            reverted_at
        ) VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&batch.id)
    .bind(&batch.conversation_id)
    .bind(&batch.tool_name)
    .bind(batch.created_at as i64)
    .bind(if batch.reverted { 1_i64 } else { 0_i64 })
    .bind(batch.reverted_at.map(|value| value as i64))
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;

    for (index, file) in batch.files.iter().enumerate() {
        let diff_json = serde_json::to_string(&file.diff)
            .map_err(|error| format!("序列化文件 diff 失败: {}", error))?;
        let (additions, deletions) = diff_counts(&file.diff);
        sqlx::query(
            r#"
            INSERT INTO file_change_files (
                batch_id,
                file_order,
                path,
                absolute_path,
                change_type,
                before_text,
                after_text,
                diff_json,
                additions,
                deletions
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&batch.id)
        .bind(index as i64)
        .bind(&file.path)
        .bind(&file.absolute_path)
        .bind(&file.change_type)
        .bind(&file.before)
        .bind(&file.after)
        .bind(diff_json)
        .bind(additions as i64)
        .bind(deletions as i64)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    }

    trim_old_batches(&mut tx, &batch.conversation_id).await?;
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn trim_old_batches(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    conversation_id: &str,
) -> Result<(), String> {
    let stale_ids = sqlx::query_scalar::<_, String>(
        r#"
        SELECT id
        FROM file_change_batches
        WHERE conversation_id = ?
        ORDER BY created_at DESC, id DESC
        LIMIT -1 OFFSET ?
        "#,
    )
    .bind(conversation_id)
    .bind(MAX_STORED_BATCHES as i64)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;

    for id in stale_ids {
        sqlx::query("DELETE FROM file_change_files WHERE batch_id = ?")
            .bind(&id)
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
        sqlx::query("DELETE FROM file_change_batches WHERE id = ?")
            .bind(&id)
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub async fn list_change_batches(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<Vec<FileChangeBatchSummary>, String> {
    let Some(conversation_id) = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return Ok(Vec::new());
    };
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let rows = sqlx::query(
        r#"
        SELECT
            b.id,
            b.conversation_id,
            b.tool_name,
            b.created_at,
            b.reverted,
            b.reverted_at,
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
    .map_err(|error| error.to_string())?;

    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row.get::<String, _>("id");
        summaries.push(FileChangeBatchSummary {
            paths: load_batch_paths(&pool, &id).await?,
            id,
            conversation_id: row.get::<String, _>("conversation_id"),
            tool_name: row.get::<String, _>("tool_name"),
            created_at: row.get::<i64, _>("created_at").max(0) as u64,
            reverted: row.get::<i64, _>("reverted") != 0,
            reverted_at: row
                .get::<Option<i64>, _>("reverted_at")
                .map(|value| value.max(0) as u64),
            file_count: row.get::<i64, _>("file_count").max(0) as usize,
            additions: row.get::<i64, _>("additions").max(0) as usize,
            deletions: row.get::<i64, _>("deletions").max(0) as usize,
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
    .map_err(|error| error.to_string())
}

pub async fn get_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    batch_id: &str,
) -> Result<FileChangeBatch, String> {
    let conversation_id = normalize_conversation_id(conversation_id)?;
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    load_change_batch(&pool, &conversation_id, batch_id).await
}

async fn load_change_batch(
    pool: &SqlitePool,
    conversation_id: &str,
    batch_id: &str,
) -> Result<FileChangeBatch, String> {
    let batch_row = sqlx::query(
        r#"
        SELECT id, conversation_id, tool_name, created_at, reverted, reverted_at
        FROM file_change_batches
        WHERE conversation_id = ? AND id = ?
        "#,
    )
    .bind(conversation_id)
    .bind(batch_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "找不到审查记录".to_string())?;

    let file_rows = sqlx::query(
        r#"
        SELECT path, absolute_path, change_type, before_text, after_text, diff_json
        FROM file_change_files
        WHERE batch_id = ?
        ORDER BY file_order ASC
        "#,
    )
    .bind(batch_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    let mut files = Vec::with_capacity(file_rows.len());
    for row in file_rows {
        let diff_json = row.get::<String, _>("diff_json");
        let diff = serde_json::from_str::<Vec<FileDiffLine>>(&diff_json)
            .map_err(|error| format!("解析文件 diff 失败: {}", error))?;
        files.push(FileChangeEntry {
            path: row.get::<String, _>("path"),
            absolute_path: row.get::<String, _>("absolute_path"),
            change_type: row.get::<String, _>("change_type"),
            before: row.get::<Option<String>, _>("before_text"),
            after: row.get::<Option<String>, _>("after_text"),
            diff,
        });
    }

    Ok(FileChangeBatch {
        id: batch_row.get::<String, _>("id"),
        conversation_id: batch_row.get::<String, _>("conversation_id"),
        tool_name: batch_row.get::<String, _>("tool_name"),
        created_at: batch_row.get::<i64, _>("created_at").max(0) as u64,
        reverted: batch_row.get::<i64, _>("reverted") != 0,
        reverted_at: batch_row
            .get::<Option<i64>, _>("reverted_at")
            .map(|value| value.max(0) as u64),
        files,
    })
}

pub async fn revert_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    batch_id: &str,
) -> Result<FileChangeBatch, String> {
    let conversation_id = normalize_conversation_id(conversation_id)?;
    let _guard = file_change_lock().lock().await;
    let pool = crate::llm::history::get_pool_with_schema(app).await?;
    let mut batch = load_change_batch(&pool, &conversation_id, batch_id).await?;
    apply_revert(&mut batch)?;
    sqlx::query("UPDATE file_change_batches SET reverted = 1, reverted_at = ? WHERE conversation_id = ? AND id = ?")
        .bind(batch.reverted_at.map(|value| value as i64))
        .bind(&conversation_id)
        .bind(batch_id)
        .execute(&pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(batch)
}

fn apply_revert(batch: &mut FileChangeBatch) -> Result<(), String> {
    if batch.reverted {
        return Err("这次变更已经回退过了".to_string());
    }

    for file in &batch.files {
        let target = absolute_path_from_change_entry(file)?;
        match &file.after {
            Some(expected) => {
                let current = fs::read_to_string(&target)
                    .map_err(|error| format!("读取当前文件失败 {}: {}", file.path, error))?;
                if &current != expected {
                    return Err(format!("文件已被后续修改，不能安全回退: {}", file.path));
                }
            }
            None => {
                if target.exists() {
                    return Err(format!("文件已被后续重新创建，不能安全回退: {}", file.path));
                }
            }
        }
    }

    for file in batch.files.iter().rev() {
        let target = absolute_path_from_change_entry(file)?;
        match &file.before {
            Some(content) => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("创建目录失败 {}: {}", file.path, error))?;
                }
                fs::write(&target, content)
                    .map_err(|error| format!("写回文件失败 {}: {}", file.path, error))?;
            }
            None => {
                if target.exists() {
                    fs::remove_file(&target)
                        .map_err(|error| format!("删除新增文件失败 {}: {}", file.path, error))?;
                }
            }
        }
    }

    batch.reverted = true;
    batch.reverted_at = Some(now_millis());
    Ok(())
}

fn validate_batch_current_state(batch: &FileChangeBatch) -> Result<(), String> {
    for file in &batch.files {
        let target = absolute_path_from_change_entry(file)?;
        match &file.before {
            Some(expected) => {
                let current = fs::read_to_string(&target)
                    .map_err(|error| format!("读取当前文件失败 {}: {}", file.path, error))?;
                if &current != expected {
                    return Err(format!("文件在写入前已变化，拒绝覆盖: {}", file.path));
                }
            }
            None => {
                if target.exists() {
                    return Err(format!("新增文件已存在，拒绝覆盖: {}", file.path));
                }
            }
        }
    }
    Ok(())
}

fn apply_batch_after(batch: &FileChangeBatch) -> Result<(), String> {
    let mut applied = Vec::<FileChangeEntry>::new();
    for file in &batch.files {
        if let Err(error) = apply_file_snapshot(file, &file.after) {
            let rollback_error = rollback_applied(&applied).err();
            return Err(match rollback_error {
                Some(rollback_error) => format!("{}；回滚失败: {}", error, rollback_error),
                None => error,
            });
        }
        applied.push(file.clone());
    }
    Ok(())
}

fn rollback_batch_after(batch: &FileChangeBatch) -> Result<(), String> {
    rollback_applied(&batch.files)
}

fn rollback_applied(files: &[FileChangeEntry]) -> Result<(), String> {
    for file in files.iter().rev() {
        apply_file_snapshot(file, &file.before)?;
    }
    Ok(())
}

fn apply_file_snapshot(file: &FileChangeEntry, snapshot: &Option<String>) -> Result<(), String> {
    let target = absolute_path_from_change_entry(file)?;
    match snapshot {
        Some(content) => {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("创建目录失败 {}: {}", file.path, error))?;
            }
            fs::write(&target, content)
                .map_err(|error| format!("写入文件失败 {}: {}", file.path, error))
        }
        None => {
            if target.exists() {
                fs::remove_file(&target)
                    .map_err(|error| format!("删除文件失败 {}: {}", file.path, error))?;
            }
            Ok(())
        }
    }
}

fn absolute_path_from_change_entry(file: &FileChangeEntry) -> Result<PathBuf, String> {
    let target = PathBuf::from(&file.absolute_path);
    if !target.is_absolute() {
        return Err(format!("文件审查记录不是绝对路径: {}", file.path));
    }
    Ok(target)
}

fn diff_lines(before: Option<&str>, after: Option<&str>) -> Vec<FileDiffLine> {
    TextDiff::from_lines(before.unwrap_or_default(), after.unwrap_or_default())
        .iter_all_changes()
        .map(|change| FileDiffLine {
            kind: match change.tag() {
                ChangeTag::Delete => "remove",
                ChangeTag::Insert => "add",
                ChangeTag::Equal => "context",
            }
            .to_string(),
            old_line: change.old_index().map(|index| index + 1),
            new_line: change.new_index().map(|index| index + 1),
            text: change
                .value()
                .trim_end_matches(&['\r', '\n'][..])
                .to_string(),
        })
        .collect()
}

fn diff_counts(diff: &[FileDiffLine]) -> (usize, usize) {
    diff.iter().fold((0, 0), |(additions, deletions), line| {
        if line.kind == "add" {
            (additions + 1, deletions)
        } else if line.kind == "remove" {
            (additions, deletions + 1)
        } else {
            (additions, deletions)
        }
    })
}
