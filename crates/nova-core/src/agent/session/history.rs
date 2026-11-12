use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Executor, Row, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use crate::core::Core;
use uuid::Uuid;

use crate::agent::session::types::{
    ConversationMeta, HistoryMessage, HistoryToolExecution,
};
use crate::provider::types::{Content, Message, Role};

/// 从首条用户消息派生会话标题：取首行、按字符截断到 24 字符。
pub fn derive_title_from_message(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    let source = if first_line.is_empty() {
        content.trim()
    } else {
        first_line
    };
    let max_chars = 24usize;
    let mut out = String::new();
    for ch in source.chars().take(max_chars) {
        out.push(ch);
    }
    if source.chars().count() > max_chars {
        format!("{}...", out)
    } else if out.is_empty() {
        "New chat".to_string()
    } else {
        out
    }
}

pub const LATEST_SCHEMA_VERSION: i64 = 2;

fn history_db_path(core: &Core) -> PathBuf {
    core.config().data_dir().join("history.db")
}

// Build sqlite database URL under core data directory.
// Format: sqlite:<path>?mode=rwc (read/write/create).
fn get_db_url(core: &Core) -> Result<String, String> {
    let db_path = history_db_path(core);

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    Ok(format!("sqlite:{}?mode=rwc", db_path.display()))
}

/// 获取当前 `Core` 实例隔离的历史库连接池（含版本化迁移与崩溃补偿扫描）。
pub async fn history_pool(core: &Core) -> Result<SqlitePool, String> {
    get_pool_with_schema(core).await
}

/// 显式关闭当前 `Core` 的 SQLite 连接池（Windows 测试清理临时目录前可用）。
pub async fn close_history_pool(core: &Core) {
    if let Some(pool) = core.db_pool_cell().get() {
        pool.close().await;
    }
}

async fn has_table_column(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    column: &str,
) -> Result<bool, String> {
    let query = format!("PRAGMA table_info({})", table);
    let rows = sqlx::query(&query)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .any(|row| row.get::<String, _>("name") == column))
}

async fn backup_db_before_migration(
    pool: &SqlitePool,
    db_path: &Path,
    from_version: i64,
) -> Result<(), String> {
    if !db_path.exists() {
        return Ok(());
    }
    let backup_name = format!("history.db.v{}.bak", from_version.max(0));
    let backup_path = db_path
        .parent()
        .map(|p| p.join(&backup_name))
        .unwrap_or_else(|| PathBuf::from(&backup_name));

    if backup_path.exists() {
        std::fs::remove_file(&backup_path)
            .map_err(|e| format!("清理旧迁移备份失败 ({}): {}", backup_path.display(), e))?;
    }

    let backup_str = backup_path.to_string_lossy().replace('\'', "''");
    let vacuum_sql = format!("VACUUM INTO '{}'", backup_str);
    if let Err(vacuum_err) = pool.execute(vacuum_sql.as_str()).await {
        tracing::warn!(
            error = %vacuum_err,
            backup = %backup_path.display(),
            "VACUUM INTO 备份失败，回退文件复制"
        );
        std::fs::copy(db_path, &backup_path)
            .map_err(|e| format!("数据库迁移前备份失败 ({}): {}", backup_path.display(), e))?;
    }
    Ok(())
}

// Create a sqlx sqlite pool for history DB.
//
// 必须启用 WAL + busy_timeout + foreign_keys：
// 连接池按 Core 实例隔离，既支持生产单库复用，又让并行测试彻底隔离无文件锁冲突。
async fn get_pool(core: &Core) -> Result<SqlitePool, String> {
    let pool = core
        .db_pool_cell()
        .get_or_try_init(|| async {
            let db_path = history_db_path(core);
            let db_url = get_db_url(core)?;
            let options = SqliteConnectOptions::from_str(&db_url)
                .map_err(|e| e.to_string())?
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
                .foreign_keys(true)
                .create_if_missing(true)
                .busy_timeout(std::time::Duration::from_secs(10));
            let pool = SqlitePoolOptions::new()
                .max_connections(5)
                .connect_with(options)
                .await
                .map_err(|e| e.to_string())?;
            run_migrations(&pool, &db_path).await?;
            let recovered =
                crate::agent::session::store::recover_interrupted_turns_on_pool(&pool, Some(core))
                    .await?;
            if recovered > 0 {
                crate::agent::utils::error_event::emit_backend_warning(
                    core,
                    "agent.session.recovery",
                    format!(
                        "检测到 {} 个未正常结束的中断回合，已在启动恢复阶段补全终态与工具结果配对",
                        recovered
                    ),
                    Some("startup.crash_recovery"),
                );
            }
            Ok::<SqlitePool, String>(pool)
        })
        .await?;
    Ok(pool.clone())
}

async fn run_migrations(pool: &SqlitePool, db_path: &Path) -> Result<(), String> {
    let current_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    if current_version > LATEST_SCHEMA_VERSION {
        return Err(format!(
            "history.db schema version v{} is newer than supported v{}; refusing to downgrade",
            current_version, LATEST_SCHEMA_VERSION
        ));
    }

    let existing_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name IN ('conversations', 'session_events', 'token_usage_log')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    if current_version < LATEST_SCHEMA_VERSION && existing_tables > 0 {
        let check: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(pool)
            .await
            .map_err(|e| format!("数据库迁移前完整性检查失败: {}", e))?;
        if !check.eq_ignore_ascii_case("ok") {
            return Err(format!("数据库迁移前完整性检查异常: {}", check));
        }
        backup_db_before_migration(pool, db_path, current_version).await?;
    }

    if current_version == LATEST_SCHEMA_VERSION {
        return Ok(());
    }

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );
        "#,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let now_ms = chrono::Utc::now().timestamp_millis();

    if current_version < 1 {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                pinned_at INTEGER,
                workspace_path TEXT,
                active_agent_id TEXT
            );
            "#,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        if !has_table_column(&mut tx, "conversations", "active_agent_id").await? {
            sqlx::query("ALTER TABLE conversations ADD COLUMN active_agent_id TEXT")
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS token_usage_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT,
                model TEXT NOT NULL,
                provider TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                cost_usd TEXT,
                source TEXT,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_token_usage_log_created
                ON token_usage_log(created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_token_usage_log_model
                ON token_usage_log(model);

            CREATE INDEX IF NOT EXISTS idx_token_usage_log_conversation
                ON token_usage_log(conversation_id);
            "#,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(crate::agent::session::store::SESSION_EVENTS_SCHEMA)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (1, '001_initial_schema', ?)
             ON CONFLICT(version) DO NOTHING",
        )
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query("PRAGMA user_version = 1")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }

    if current_version < 2 {
        if !has_table_column(&mut tx, "session_events", "trace_id").await? {
            sqlx::query("ALTER TABLE session_events ADD COLUMN trace_id TEXT")
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_session_events_trace
                ON session_events(trace_id);
            CREATE INDEX IF NOT EXISTS idx_session_events_type_turn
                ON session_events(conversation_id, event_type, turn_id);
            "#,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (2, '002_trace_id_and_diagnostics', ?)
             ON CONFLICT(version) DO NOTHING",
        )
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query("PRAGMA user_version = 2")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

// Public helper used by command handlers:
// open DB pool and guarantee schema is ready before query.
pub async fn get_pool_with_schema(core: &Core) -> Result<SqlitePool, String> {
    get_pool(core).await
}

fn resolved_conversation_title(current_title: &str, first_user_message: Option<&str>) -> String {
    let trimmed = current_title.trim();
    if !trimmed.is_empty() && trimmed != "New chat" && trimmed != "CLI session" && trimmed != "CLI headless" {
        return trimmed.to_string();
    }

    first_user_message
        .map(derive_title_from_message)
        .unwrap_or_else(|| "新对话".to_string())
}

fn sanitize_export_file_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            ch if ch.is_control() => '-',
            ch => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch| ch == ' ' || ch == '.')
        .chars()
        .take(80)
        .collect::<String>();

    if sanitized.is_empty() {
        "conversation".to_string()
    } else {
        sanitized
    }
}

fn ensure_json_export_format(format: &str) -> Result<(), String> {
    match format.trim().to_ascii_lowercase().as_str() {
        "json" => Ok(()),
        other => Err(format!("unsupported export format: {}", other)),
    }
}

fn export_dir(core: &Core) -> Result<PathBuf, String> {
    let dir = core.config().download_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn build_export_path(
    core: &Core,
    title: &str,
    exported_at: chrono::DateTime<chrono::Utc>,
    extension: &str,
) -> Result<PathBuf, String> {
    let mut path = export_dir(core)?;
    path.push(format!(
        "nova-{}-{}.{}",
        sanitize_export_file_name(title),
        exported_at.format("%Y%m%d-%H%M%S"),
        extension
    ));
    Ok(path)
}

struct ConversationExportData {
    title: String,
    messages: Vec<HistoryMessage>,
}

async fn load_conversation_export_data(
    core: &Core,
    conversation_id: &str,
) -> Result<ConversationExportData, String> {
    let pool = get_pool_with_schema(core).await?;

    let title_row: Option<String> =
        sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let title_row = title_row.ok_or_else(|| "conversation not found".to_string())?;

    let messages = load_history(core, conversation_id).await?;
    // 标题兜底：占位标题时从事件投影出的首条用户消息派生。
    let first_user = messages
        .iter()
        .find(|m| m.role.eq_ignore_ascii_case("user"))
        .map(|m| m.content.clone());
    let title = resolved_conversation_title(&title_row, first_user.as_deref());

    Ok(ConversationExportData { title, messages })
}

async fn ensure_conversation_exists(core: &Core, conversation_id: &str) -> Result<(), String> {
    let pool = get_pool_with_schema(core).await?;
    let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM conversations WHERE id = ? LIMIT 1")
        .bind(conversation_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .is_some();

    if exists {
        Ok(())
    } else {
        Err("conversation not found".to_string())
    }
}

fn chromium_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect()
    }

    #[cfg(not(target_os = "windows"))]
    {
        vec![
            PathBuf::from("google-chrome"),
            PathBuf::from("microsoft-edge"),
            PathBuf::from("chromium-browser"),
            PathBuf::from("chromium"),
        ]
    }
}

async fn print_html_to_pdf(html_path: &Path, output_path: &Path) -> Result<(), String> {
    let candidates = chromium_candidates();
    if candidates.is_empty() {
        return Err("未找到 Edge/Chrome/Chromium，无法将已渲染 HTML 打印为 PDF。".to_string());
    }

    let mut errors = Vec::new();
    for browser in candidates {
        let output = tokio::process::Command::new(&browser)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-pdf-header-footer")
            .arg(format!("--print-to-pdf={}", output_path.display()))
            .arg(html_path.as_os_str())
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() && output_path.exists() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                errors.push(format!(
                    "{} exited with {}. stderr: {} stdout: {}",
                    browser.display(),
                    output.status,
                    stderr.trim(),
                    stdout.trim()
                ));
            }
            Err(err) => errors.push(format!("{}: {}", browser.display(), err)),
        }
    }

    Err(format!("PDF 导出失败：{}", errors.join("; ")))
}

// Create a new conversation row with generated UUID and optional title.
// workspace_path: 该会话绑定的项目工作区目录；None 时使用内置默认工作区。
pub async fn create_conversation(
    core: &Core,
    title: Option<String>,
    workspace_path: Option<String>,
) -> Result<ConversationMeta, String> {
    let pool = get_pool_with_schema(core).await?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let conv_title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_default();

    // 没传 workspace_path 或传空，就用内置默认工作区（app_data/workspace）下的独立子目录。
    let ws_path = match workspace_path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        Some(p) => p,
        None => {
            let base = crate::services::workspace::default_workspace_root(core)?;
            let conv_dir = base.join(&id);
            std::fs::create_dir_all(&conv_dir)
                .map_err(|e| format!("创建对话工作区失败: {}", e))?;
            let canonical = conv_dir.canonicalize()
                .map_err(|e| format!("无法解析对话工作区: {}", e))?;
            crate::services::workspace::display_path_string(&canonical)
        }
    };

    // 会话元数据单行写入；消息内容后续由会话事件日志承担。
    sqlx::query(
        "INSERT INTO conversations (id, title, created_at, updated_at, workspace_path) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&conv_title)
    .bind(now)
    .bind(now)
    .bind(&ws_path)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    // 写入进程内缓存，供同步热路径读取。
    crate::services::workspace::cache_conversation_workspace(&id, &ws_path);

    Ok(ConversationMeta {
        id,
        title: conv_title,
        updated_at: now,
        pinned_at: None,
        workspace_path: Some(ws_path),
        active_agent_id: None,
    })
}

// List conversations ordered by latest update time.
pub async fn list_conversations(core: &Core) -> Result<Vec<ConversationMeta>, String> {
    let pool = get_pool_with_schema(core).await?;

    let rows = sqlx::query(
        r#"
        SELECT
            c.id,
            c.title,
            c.updated_at,
            c.pinned_at,
            c.workspace_path,
            c.active_agent_id
        FROM conversations c
        ORDER BY
            CASE WHEN c.pinned_at IS NULL THEN 1 ELSE 0 END ASC,
            c.pinned_at DESC,
            c.updated_at DESC
        "#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let items: Vec<ConversationMeta> = rows
        .into_iter()
        .map(|row| {
            let ws_path = row
                .get::<Option<String>, _>("workspace_path")
                .filter(|p| !p.trim().is_empty());
            ConversationMeta {
                id: row.get::<String, _>("id"),
                // 标题在首条用户消息发送时已派生落库，这里只做占位规范化。
                title: resolved_conversation_title(&row.get::<String, _>("title"), None),
                updated_at: row.get::<i64, _>("updated_at"),
                pinned_at: row.get::<Option<i64>, _>("pinned_at"),
                workspace_path: ws_path,
                active_agent_id: row
                    .get::<Option<String>, _>("active_agent_id")
                    .filter(|v| !v.trim().is_empty()),
            }
        })
        .collect();

    // 批量刷新进程内缓存，供同步热路径读取。
    let cache_entries: Vec<(String, Option<String>)> = items
        .iter()
        .map(|c| (c.id.clone(), c.workspace_path.clone()))
        .collect();
    crate::services::workspace::refresh_workspace_cache(&cache_entries).await;

    // 会话级智能体缓存同样批量刷新（写穿透模式的兜底刷新点之一）。
    let agent_entries: Vec<(String, Option<String>)> = rows_agent_entries(&items);
    crate::agent::capabilities::agent_bundles::refresh_conversation_agent_cache(&agent_entries);

    Ok(items)
}

// 从行数据提取 (会话id, 智能体id) 对，供缓存批量刷新。
fn rows_agent_entries(items: &[ConversationMeta]) -> Vec<(String, Option<String>)> {
    items
        .iter()
        .map(|c| (c.id.clone(), c.active_agent_id.clone()))
        .collect()
}

/// 设置/移除会话挂载的智能体（bundle_id 为 None = 回到默认 Nova）。
/// 写库后同步更新进程内缓存（写穿透）。
pub async fn set_conversation_agent(
    core: &Core,
    conversation_id: &str,
    bundle_id: Option<&str>,
) -> Result<(), String> {
    let pool = get_pool_with_schema(core).await?;
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return Err("conversation_id is required".to_string());
    }

    let agent = bundle_id.map(str::trim).filter(|v| !v.is_empty());
    sqlx::query("UPDATE conversations SET active_agent_id = ? WHERE id = ?")
        .bind(agent)
        .bind(normalized)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    // 写穿透：立刻同步缓存，provider 同步热路径马上可见。
    crate::agent::capabilities::agent_bundles::cache_conversation_agent(normalized, agent);
    Ok(())
}

/// 读取会话挂载的智能体 id（无会话/未挂载返回 None）。
pub async fn get_conversation_agent(
    core: &Core,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    let pool = get_pool_with_schema(core).await?;
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return Ok(None);
    }

    let agent: Option<Option<String>> =
        sqlx::query_scalar("SELECT active_agent_id FROM conversations WHERE id = ?")
            .bind(normalized)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(agent.flatten().filter(|v| !v.trim().is_empty()))
}

/// 删除 bundle 后清空所有引用它的会话（回到默认 Nova），并刷新缓存。
pub async fn clear_conversation_agent_references(
    core: &Core,
    bundle_id: &str,
) -> Result<Vec<String>, String> {
    let pool = get_pool_with_schema(core).await?;
    let normalized = bundle_id.trim();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }

    // 先找出受影响会话，再清引用（不 bump updated_at，避免会话列表顺序被打乱）。
    let affected: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM conversations WHERE active_agent_id = ?",
    )
    .bind(normalized)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("UPDATE conversations SET active_agent_id = NULL WHERE active_agent_id = ?")
        .bind(normalized)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    for conversation_id in &affected {
        crate::agent::capabilities::agent_bundles::cache_conversation_agent(conversation_id, None);
    }

    Ok(affected)
}

pub async fn set_conversation_pinned(
    core: &Core,
    conversation_id: &str,
    pinned: bool,
) -> Result<(), String> {
    let pool = get_pool_with_schema(core).await?;
    let pinned_at = pinned.then(|| chrono::Utc::now().timestamp());

    let result = sqlx::query("UPDATE conversations SET pinned_at = ? WHERE id = ?")
        .bind(pinned_at)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    if result.rows_affected() == 0 {
        return Err("conversation not found".to_string());
    }

    Ok(())
}

pub async fn export_conversation(
    core: &Core,
    conversation_id: &str,
    format: &str,
) -> Result<String, String> {
    ensure_json_export_format(format)?;
    let data = load_conversation_export_data(core, conversation_id).await?;
    let exported_at = chrono::Utc::now();

    let output_path = build_export_path(core, &data.title, exported_at, "json")?;
    let payload = serde_json::json!({
        "id": conversation_id,
        "title": data.title,
        "exportedAt": exported_at.to_rfc3339(),
        "messages": data.messages,
    });
    let body = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    tokio::fs::write(&output_path, body)
        .await
        .map_err(|e| e.to_string())?;

    Ok(output_path.display().to_string())
}

pub async fn export_rendered_conversation_pdf(
    core: &Core,
    conversation_id: &str,
    title: &str,
    html: &str,
) -> Result<String, String> {
    if html.trim().is_empty() {
        return Err("rendered html is empty".to_string());
    }

    ensure_conversation_exists(core, conversation_id).await?;
    let exported_at = chrono::Utc::now();
    let output_path = build_export_path(core, title, exported_at, "pdf")?;

    let mut temp_dir = core.config().cache_dir();
    temp_dir.push("exports");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
    temp_dir.push(format!(
        "{}-{}.html",
        conversation_id,
        exported_at.format("%Y%m%d-%H%M%S")
    ));

    tokio::fs::write(&temp_dir, html)
        .await
        .map_err(|e| e.to_string())?;
    let print_result = print_html_to_pdf(&temp_dir, &output_path).await;
    let _ = tokio::fs::remove_file(&temp_dir).await;
    print_result?;

    Ok(output_path.display().to_string())
}

/// 会话活跃度与标题维护（事件日志时代的持久化入口）：
/// 刷新 updated_at；user_text 非空且标题仍为占位时从首条用户消息派生标题。
pub async fn refresh_conversation_activity(
    core: &Core,
    conversation_id: &str,
    user_text: Option<&str>,
) -> Result<(), String> {
    let pool = get_pool_with_schema(core).await?;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    let Some(text) = user_text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(());
    };

    let current_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    let should_update = current_title
        .as_deref()
        .map(|title| {
            let trimmed = title.trim();
            trimmed.is_empty() || trimmed == "New chat" || trimmed == "新对话" || trimmed == "CLI session" || trimmed == "CLI headless"
        })
        .unwrap_or(false);
    if !should_update {
        return Ok(());
    }

    let new_title = derive_title_from_message(text);
    sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(&new_title)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let title_event = crate::agent::session::SessionEvent::TitleChanged { title: new_title };
    if let Err(error) =
        crate::agent::session::append_event(core, conversation_id, None, &title_event).await
    {
        tracing::warn!(error = %error, "title_changed event append failed");
    }
    Ok(())
}

/// 事件驱动的纯文本消息追加（分支转存等场景）：写事件日志并维护标题/活跃度。
pub async fn append_plain_chat_message(
    core: &Core,
    conversation_id: &str,
    role: &str,
    content: &str,
) -> Result<(), String> {
    let message_role = if role.eq_ignore_ascii_case("assistant") {
        Role::Assistant
    } else {
        Role::User
    };
    let message = Message {
        role: message_role.clone(),
        content: Content::Text(content.to_string()),
    };
    let event = crate::agent::session::SessionEvent::from_model_message(message);
    crate::agent::session::append_event(core, conversation_id, None, &event).await?;
    let user_text = matches!(message_role, Role::User).then_some(content);
    refresh_conversation_activity(core, conversation_id, user_text).await
}

/// 事件驱动的历史重置（编辑消息重发用）：清空该会话事件流，
/// 再把传入消息序列重写为事件（用户消息保留附件，助手消息保留 token/成本）。
pub async fn reset_conversation_messages(
    core: &Core,
    conversation_id: &str,
    messages: &[HistoryMessage],
) -> Result<(), String> {
    crate::agent::session::delete_events(core, conversation_id).await?;

    let mut events = Vec::with_capacity(messages.len());
    for message in messages {
        let role = if message.role.eq_ignore_ascii_case("assistant") {
            Role::Assistant
        } else {
            Role::User
        };
        let model_message = Message {
            role: role.clone(),
            content: Content::Text(message.content.clone()),
        };
        let event = match role {
            Role::User => crate::agent::session::SessionEvent::UserMessage {
                message: model_message,
                attachments: message.attachments.clone(),
            },
            Role::Assistant => crate::agent::session::SessionEvent::AssistantMessage {
                message: model_message,
                token_usage: message.token_usage,
                cost: message.cost.clone(),
            },
        };
        events.push(event);
    }
    crate::agent::session::append_events(core, conversation_id, None, &events).await?;

    let first_user = messages
        .iter()
        .find(|m| m.role.eq_ignore_ascii_case("user"))
        .map(|m| m.content.as_str());
    refresh_conversation_activity(core, conversation_id, first_user).await
}

// Load all persisted messages for a conversation in stable chronological order.
// 纯事件投影：从会话事件日志渲染（旧数据不兼容，无事件的会话返回空）。
pub async fn load_history(
    core: &Core,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String> {
    let events = crate::agent::session::load_events(core, conversation_id).await?;
    Ok(crate::agent::session::projection::render_ui_history(&events))
}

pub async fn load_conversation_tool_logs(
    core: &Core,
    conversation_id: &str,
) -> Result<Vec<HistoryToolExecution>, String> {
    // 纯事件投影：从 ToolCall/ToolResult 事件配对渲染。
    let events = crate::agent::session::load_events(core, conversation_id).await?;
    Ok(crate::agent::session::projection::render_tool_logs(&events))
}

// Clear history data.
// - with conversation_id: clear the scoped conversation's persisted data
// - without conversation_id: clear all persisted history and conversation rows
pub async fn clear_history(core: &Core, conversation_id: Option<String>) -> Result<(), String> {
    let pool = get_pool_with_schema(core).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    if let Some(id) = conversation_id {
        sqlx::query("DELETE FROM session_events WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE token_usage_log SET conversation_id = NULL WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::agent::capabilities::session_files::delete_all_session_files(core, &id)?;
        let _ = crate::agent::capabilities::plan_files::delete_conversation_plan(core, Some(&id));
        crate::services::unified_exec::close_conversation(Some(&id)).await;
        cleanup_collaboration_state(&id);
        core.hooks().stop_user_terminal_session(&id);
        // 内存级 per-conversation 缓存统一回收（注册表单一入口）。
        crate::agent::utils::cache_registry::clear_conversation_caches(Some(&id));
    } else {
        sqlx::query("DELETE FROM session_events")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE token_usage_log SET conversation_id = NULL")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::agent::capabilities::session_files::delete_all_session_files_all(core)?;
        crate::services::unified_exec::close_all().await;
        cleanup_all_collaboration_state();
        core.hooks().close_all_user_terminal_sessions();
        // 全量清除路径：所有会话级内存缓存一并清空。
        crate::agent::utils::cache_registry::clear_conversation_caches(None);
    }

    Ok(())
}

/// 会话删除时一并回收进程内的协同状态（看板 / 图谱 / 子智能体句柄）。
/// 不清理的话这三张全局 Map 会随着"建会话-删会话"无限增长，且落盘文件只增不减。
fn cleanup_collaboration_state(conversation_id: &str) {
    crate::agent::orchestration::board::global_message_board()
        .remove_conversation(conversation_id);
    crate::agent::orchestration::graph::global_graph_store()
        .remove_conversation(conversation_id);
    crate::agent::orchestration::registry::global_agent_registry()
        .remove_conversation(conversation_id);
}

fn cleanup_all_collaboration_state() {
    crate::agent::orchestration::board::global_message_board().remove_all();
    crate::agent::orchestration::graph::global_graph_store().remove_all();
    crate::agent::orchestration::registry::global_agent_registry().remove_all();
}

// Delete one conversation and all dependent rows.
pub async fn delete_conversation(core: &Core, conversation_id: &str) -> Result<(), String> {
    let pool = get_pool_with_schema(core).await?;

    // Check if the conversation has an auto-generated workspace to clean up
    let ws_path_opt: Option<String> = sqlx::query_scalar("SELECT workspace_path FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();

    if let Some(ws_path) = ws_path_opt {
        if let Ok(default_root) = crate::services::workspace::default_workspace_root(core) {
            let auto_dir = default_root.join(conversation_id);
            if let Ok(canonical_auto) = auto_dir.canonicalize() {
                if ws_path == crate::services::workspace::display_path_string(&canonical_auto) {
                    let _ = std::fs::remove_dir_all(&auto_dir);
                }
            }
        }
    }

    sqlx::query("DELETE FROM session_events WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("UPDATE token_usage_log SET conversation_id = NULL WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    crate::agent::capabilities::session_files::delete_all_session_files(core, conversation_id)?;
    let _ = crate::agent::capabilities::plan_files::delete_conversation_plan(core, Some(conversation_id));
    crate::services::unified_exec::close_conversation(Some(conversation_id)).await;
    cleanup_collaboration_state(conversation_id);
    core.hooks().stop_user_terminal_session(conversation_id);

    // 内存级 per-conversation 缓存统一回收（注册表单一入口）。
    crate::agent::utils::cache_registry::clear_conversation_caches(Some(conversation_id));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CoreConfig;
    use crate::events::NovaEvent;
    use crate::provider::types::{Content, ContentBlock, Message, Role};

    #[tokio::test]
    async fn fresh_db_runs_explicit_migrations_with_foreign_keys_enabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let core = Core::new(CoreConfig::new(dir.path().to_path_buf()), 32);

        let pool = history_pool(&core).await.expect("pool initialized");
        let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("user_version");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("foreign_keys");
        let migration_versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version ASC")
                .fetch_all(&pool)
                .await
                .expect("schema_migrations");

        assert_eq!(user_version, LATEST_SCHEMA_VERSION);
        assert_eq!(foreign_keys, 1);
        assert_eq!(migration_versions, vec![1, 2]);

        close_history_pool(&core).await;
    }

    #[tokio::test]
    async fn migrates_legacy_v0_db_creates_backup_and_preserves_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("history.db");

        // 构造缺失 active_agent_id 与 trace_id 列的旧版 v0 数据库。
        let url = format!("sqlite:{}?mode=rwc", db_path.display());
        let opts = SqliteConnectOptions::from_str(&url)
            .expect("opts")
            .create_if_missing(true);
        let legacy_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("legacy pool");
        sqlx::query(
            "CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                pinned_at INTEGER,
                workspace_path TEXT
            );",
        )
        .execute(&legacy_pool)
        .await
        .expect("create legacy conversations");
        sqlx::query(
            "CREATE TABLE session_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                turn_id TEXT,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE (conversation_id, seq)
            );",
        )
        .execute(&legacy_pool)
        .await
        .expect("create legacy session_events");
        sqlx::query(
            "INSERT INTO conversations (id, title, created_at, updated_at)
             VALUES ('conv-legacy', 'Legacy Chat', 100, 100)",
        )
        .execute(&legacy_pool)
        .await
        .expect("insert legacy row");
        legacy_pool.close().await;

        let core = Core::new(CoreConfig::new(dir.path().to_path_buf()), 32);
        let list = list_conversations(&core).await.expect("list after migration");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "conv-legacy");
        assert!(dir.path().join("history.db.v0.bak").exists());

        let pool = history_pool(&core).await.expect("migrated pool");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("user_version");
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        close_history_pool(&core).await;
    }

    #[tokio::test]
    async fn startup_recovery_repairs_unclosed_turn_and_dangling_tool_call() {
        let dir = tempfile::tempdir().expect("tempdir");

        // 1. 首轮进程：写入未闭合 TurnStart + Assistant ToolUse + ToolCall 后“崩溃”。
        {
            let core1 = Core::new(CoreConfig::new(dir.path().to_path_buf()), 64);
            let conv = create_conversation(&core1, Some("Crash Test".to_string()), None)
                .await
                .expect("create conv");

            let crash_events = vec![
                crate::agent::session::SessionEvent::TurnStart {
                    turn_id: "turn-crash".to_string(),
                    trace_id: Some("trace-crash".to_string()),
                },
                crate::agent::session::SessionEvent::UserMessage {
                    message: Message {
                        role: Role::User,
                        content: Content::Text("run command".to_string()),
                    },
                    attachments: None,
                },
                crate::agent::session::SessionEvent::AssistantMessage {
                    message: Message {
                        role: Role::Assistant,
                        content: Content::Blocks(vec![ContentBlock::ToolUse {
                            id: "call-dangling".to_string(),
                            name: "exec_command".to_string(),
                            input: serde_json::json!({"cmd": "echo hi"}),
                        }]),
                    },
                    token_usage: None,
                    cost: None,
                },
                crate::agent::session::SessionEvent::ToolCall {
                    call_id: "call-dangling".to_string(),
                    tool_name: "exec_command".to_string(),
                    input: r#"{"cmd":"echo hi"}"#.to_string(),
                    turn_id: Some("turn-crash".to_string()),
                    trace_id: Some("trace-crash".to_string()),
                },
            ];
            crate::agent::session::append_events(&core1, &conv.id, Some("turn-crash"), &crash_events)
                .await
                .expect("append pre-crash events");
            close_history_pool(&core1).await;
        }

        // 2. 重启后新 Core 实例打开库：应自动检测并补偿修复未闭合回合且触发后端告警。
        let core2 = Core::new(CoreConfig::new(dir.path().to_path_buf()), 64);
        let mut rx = core2.subscribe();
        let convs = list_conversations(&core2).await.expect("list convs");
        let conv_id = convs[0].id.clone();

        let traces = crate::api::history::get_conversation_turn_traces(&core2, conv_id.clone())
            .await
            .expect("traces");
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].stop_reason.as_deref(), Some("interrupted"));
        assert!(traces[0].ended_at.is_some());
        assert_eq!(traces[0].trace_id.as_deref(), Some("trace-crash"));

        let tool_logs = load_conversation_tool_logs(&core2, &conv_id)
            .await
            .expect("tool logs");
        assert_eq!(tool_logs.len(), 1);
        assert_eq!(tool_logs[0].status, "error");
        assert!(tool_logs[0].result.contains("Interrupted"));

        let stored = crate::agent::session::load_events(&core2, &conv_id)
            .await
            .expect("load events");
        let ctx = crate::agent::session::projection::reconstruct_model_context(&stored);
        let has_matching_tool_result = ctx.iter().any(|msg| match &msg.content {
            Content::Blocks(blocks) => blocks.iter().any(|b| {
                matches!(
                    b,
                    ContentBlock::ToolResult {
                        tool_use_id,
                        is_error: true,
                        ..
                    } if tool_use_id == "call-dangling"
                )
            }),
            _ => false,
        });
        assert!(
            has_matching_tool_result,
            "recovered context must pair dangling ToolUse with synthetic error ToolResult"
        );

        let mut saw_recovery_warning = false;
        while let Ok(ev) = rx.try_recv() {
            if let NovaEvent::BackendWarning(w) = ev {
                if w.source == "agent.session.recovery" {
                    saw_recovery_warning = true;
                }
            }
        }
        assert!(saw_recovery_warning, "expected backend-warning on startup recovery");

        close_history_pool(&core2).await;
    }

    #[tokio::test]
    async fn store_write_failure_emits_backend_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let core = Core::new(CoreConfig::new(dir.path().to_path_buf()), 32);
        let pool = history_pool(&core).await.expect("pool");
        let mut rx = core.subscribe();

        // 故意破坏目标表使写入必然失败，验证不会静默吞掉。
        sqlx::query("DROP TABLE session_events")
            .execute(&pool)
            .await
            .expect("drop session_events");

        let res = crate::agent::session::append_event(
            &core,
            "conv-broken",
            Some("turn-1"),
            &crate::agent::session::SessionEvent::TurnStart {
                turn_id: "turn-1".to_string(),
                trace_id: None,
            },
        )
        .await;
        assert!(res.is_err());

        let mut saw_store_warning = false;
        while let Ok(ev) = rx.try_recv() {
            if let NovaEvent::BackendWarning(w) = ev {
                if w.source == "agent.session.store" {
                    saw_store_warning = true;
                }
            }
        }
        assert!(saw_store_warning, "store failure must emit backend-warning");

        close_history_pool(&core).await;
    }
}
