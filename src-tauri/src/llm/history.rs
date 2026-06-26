use serde_json::Value;
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::llm::commands::memory;
use crate::llm::commands::types::{
    ConversationMeta, GlobalMemoryEntry, HistoryMessage, HistoryToolExecution,
};
use crate::llm::types::{Content, ContentBlock, Message, Role};

// Build sqlite database URL under app data directory.
// Format: sqlite:<path>?mode=rwc (read/write/create).
fn get_db_url(app: &AppHandle) -> Result<String, String> {
    let db_path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("history.db");

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    Ok(format!("sqlite:{}?mode=rwc", db_path.display()))
}

// Create a sqlx sqlite pool for history DB.
async fn get_pool(app: &AppHandle) -> Result<SqlitePool, String> {
    let db_url = get_db_url(app)?;
    SqlitePool::connect(&db_url)
        .await
        .map_err(|e| e.to_string())
}

// Ensure required schema exists.
async fn ensure_schema(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            pinned_at INTEGER,
            workspace_path TEXT
        );

        CREATE TABLE IF NOT EXISTS conversation_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            reasoning TEXT,
            attachments_json TEXT,
            token_usage INTEGER,
            cost_json TEXT,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (conversation_id) REFERENCES conversations(id)
        );

        CREATE TABLE IF NOT EXISTS conversation_tool_logs (
            conversation_id TEXT NOT NULL,
            log_id TEXT NOT NULL,
            turn_id TEXT,
            tool_name TEXT NOT NULL,
            input_text TEXT NOT NULL,
            result_text TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (conversation_id, log_id),
            FOREIGN KEY (conversation_id) REFERENCES conversations(id)
        );

        CREATE TABLE IF NOT EXISTS conversation_memory (
            conversation_id TEXT PRIMARY KEY,
            summary TEXT NOT NULL,
            key_facts_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (conversation_id) REFERENCES conversations(id)
        );

        CREATE TABLE IF NOT EXISTS conversation_compact_boundaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL,
            context_text TEXT NOT NULL,
            summary TEXT NOT NULL,
            key_facts_json TEXT NOT NULL,
            recent_limit INTEGER NOT NULL,
            omitted_message_count INTEGER NOT NULL,
            total_message_count INTEGER NOT NULL,
            estimated_tokens INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (conversation_id) REFERENCES conversations(id)
        );

        CREATE TABLE IF NOT EXISTS conversation_turn_snapshots (
            conversation_id TEXT PRIMARY KEY,
            snapshot_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        "#,
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

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
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

// Public helper used by command handlers:
// open DB pool and guarantee schema is ready before query.
pub async fn get_pool_with_schema(app: &AppHandle) -> Result<SqlitePool, String> {
    let pool = get_pool(app).await?;
    ensure_schema(&pool).await?;
    Ok(pool)
}

pub async fn upsert_global_memory(
    app: &AppHandle,
    content: &str,
    kind: Option<&str>,
    source: Option<&str>,
) -> Result<GlobalMemoryEntry, String> {
    crate::llm::services::memory_dir::upsert_global_memory(app, content, kind, source).await
}

pub async fn list_global_memory(
    app: &AppHandle,
    limit: Option<i64>,
) -> Result<Vec<GlobalMemoryEntry>, String> {
    crate::llm::services::memory_dir::list_global_memory(app, limit).await
}

pub async fn delete_global_memory(app: &AppHandle, id: i64) -> Result<bool, String> {
    crate::llm::services::memory_dir::delete_global_memory(app, id).await
}

pub async fn clear_global_memory(app: &AppHandle) -> Result<i64, String> {
    crate::llm::services::memory_dir::clear_global_memory(app).await
}

async fn conversation_exists(pool: &SqlitePool, conversation_id: &str) -> Result<bool, String> {
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return Ok(false);
    }

    let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?)")
        .bind(normalized)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(exists != 0)
}

fn is_actual_user_message(message: &Message) -> bool {
    if message.role != Role::User {
        return false;
    }

    match &message.content {
        Content::Text(text) => !text.trim().is_empty(),
        Content::Blocks(blocks) => blocks.iter().any(|block| {
            matches!(
                block,
                ContentBlock::Text { .. } | ContentBlock::Image { .. }
            )
        }),
    }
}

fn snapshot_before_user_ordinal(snapshot: &[Message], user_ordinal: usize) -> Vec<Message> {
    if user_ordinal == 0 {
        return Vec::new();
    }

    let mut seen_users = 0usize;
    for (index, message) in snapshot.iter().enumerate() {
        if is_actual_user_message(message) {
            seen_users += 1;
            if seen_users == user_ordinal {
                return snapshot[..index].to_vec();
            }
        }
    }

    snapshot.to_vec()
}

fn resolved_conversation_title(current_title: &str, first_user_message: Option<&str>) -> String {
    let trimmed = current_title.trim();
    if !trimmed.is_empty() && trimmed != "New chat" {
        return trimmed.to_string();
    }

    first_user_message
        .map(memory::derive_title_from_message)
        .unwrap_or_else(|| trimmed.to_string())
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

fn export_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn build_export_path(
    app: &AppHandle,
    title: &str,
    exported_at: chrono::DateTime<chrono::Utc>,
    extension: &str,
) -> Result<PathBuf, String> {
    let mut path = export_dir(app)?;
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
    app: &AppHandle,
    conversation_id: &str,
) -> Result<ConversationExportData, String> {
    let pool = get_pool_with_schema(app).await?;

    let row = sqlx::query(
        r#"
        SELECT
            c.title,
            (
                SELECT m.content
                FROM conversation_messages m
                WHERE m.conversation_id = c.id AND m.role = 'user'
                ORDER BY m.created_at ASC, m.id ASC
                LIMIT 1
            ) AS first_user_content
        FROM conversations c
        WHERE c.id = ?
        "#,
    )
    .bind(conversation_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "conversation not found".to_string())?;

    let title = resolved_conversation_title(
        &row.get::<String, _>("title"),
        row.get::<Option<String>, _>("first_user_content")
            .as_deref(),
    );
    let messages = load_history(app, conversation_id).await?;

    Ok(ConversationExportData { title, messages })
}

async fn ensure_conversation_exists(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
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

    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect()
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
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
    app: &AppHandle,
    title: Option<String>,
    workspace_path: Option<String>,
) -> Result<ConversationMeta, String> {
    let pool = get_pool_with_schema(app).await?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let conv_title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_default();

    // 没传 workspace_path 或传空，就用内置默认工作区（app_data/workspace）。
    let ws_path = match workspace_path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        Some(p) => p,
        None => crate::command::workspace::default_workspace_root(app)
            .map(|path| crate::command::workspace::display_path_string(&path))?,
    };

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
    crate::command::workspace::cache_conversation_workspace(&id, &ws_path);

    Ok(ConversationMeta {
        id,
        title: conv_title,
        updated_at: now,
        pinned_at: None,
        workspace_path: Some(ws_path),
    })
}

// List conversations ordered by latest update time.
pub async fn list_conversations(app: &AppHandle) -> Result<Vec<ConversationMeta>, String> {
    let pool = get_pool_with_schema(app).await?;

    let rows = sqlx::query(
        r#"
        SELECT
            c.id,
            c.title,
            c.updated_at,
            c.pinned_at,
            c.workspace_path,
            (
                SELECT m.content
                FROM conversation_messages m
                WHERE m.conversation_id = c.id AND m.role = 'user'
                ORDER BY m.created_at ASC, m.id ASC
                LIMIT 1
            ) AS first_user_content
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
                title: resolved_conversation_title(
                    &row.get::<String, _>("title"),
                    row.get::<Option<String>, _>("first_user_content")
                        .as_deref(),
                ),
                updated_at: row.get::<i64, _>("updated_at"),
                pinned_at: row.get::<Option<i64>, _>("pinned_at"),
                workspace_path: ws_path,
            }
        })
        .collect();

    // 批量刷新进程内缓存，供同步热路径读取。
    let cache_entries: Vec<(String, Option<String>)> = items
        .iter()
        .map(|c| (c.id.clone(), c.workspace_path.clone()))
        .collect();
    crate::command::workspace::refresh_workspace_cache(&cache_entries).await;

    Ok(items)
}

pub async fn set_conversation_pinned(
    app: &AppHandle,
    conversation_id: &str,
    pinned: bool,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
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
    app: &AppHandle,
    conversation_id: &str,
    format: &str,
) -> Result<String, String> {
    ensure_json_export_format(format)?;
    let data = load_conversation_export_data(app, conversation_id).await?;
    let exported_at = chrono::Utc::now();

    let output_path = build_export_path(app, &data.title, exported_at, "json")?;
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
    app: &AppHandle,
    conversation_id: &str,
    title: &str,
    html: &str,
) -> Result<String, String> {
    if html.trim().is_empty() {
        return Err("rendered html is empty".to_string());
    }

    ensure_conversation_exists(app, conversation_id).await?;
    let exported_at = chrono::Utc::now();
    let output_path = build_export_path(app, title, exported_at, "pdf")?;

    let mut temp_dir = app
        .path()
        .app_cache_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| e.to_string())?;
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

// Load all persisted messages for a conversation in stable chronological order.
// cost_json is parsed into JSON when possible; malformed JSON is safely ignored.
pub async fn load_history(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String> {
    let pool = get_pool_with_schema(app).await?;

    let rows = sqlx::query(
        "SELECT role, content, reasoning, attachments_json, token_usage, cost_json FROM conversation_messages WHERE conversation_id = ? ORDER BY created_at ASC, id ASC",
    )
    .bind(conversation_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let result = rows
        .into_iter()
        .map(|row| HistoryMessage {
            role: row.get::<String, _>("role"),
            content: row.get::<String, _>("content"),
            reasoning: row.get::<Option<String>, _>("reasoning"),
            attachments: row
                .get::<Option<String>, _>("attachments_json")
                .and_then(|s| serde_json::from_str(&s).ok()),
            token_usage: row.get::<Option<i64>, _>("token_usage"),
            cost: row
                .get::<Option<String>, _>("cost_json")
                .and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        })
        .collect();

    Ok(result)
}

// Append one message to conversation history and maintain related metadata:
// 1) insert message row
// 2) auto-derive title on first user message when title is still default
// 3) update conversation updated_at
// 4) refresh conversation memory summary/facts
pub async fn append_history(
    app: &AppHandle,
    conversation_id: &str,
    message: HistoryMessage,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized_conversation_id = conversation_id.trim();

    // Stream callbacks may outlive a conversation (reload/delete/clear); skip stale writes.
    if !conversation_exists(&pool, normalized_conversation_id).await? {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    let role = message.role.clone();
    let content = message.content.clone();
    let reasoning = message
        .reasoning
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let attachments_json = message
        .attachments
        .and_then(|v| serde_json::to_string(&v).ok());
    let token_usage = message.token_usage;
    let cost_json = message.cost.and_then(|v| serde_json::to_string(&v).ok());

    sqlx::query(
        "INSERT INTO conversation_messages (conversation_id, role, content, reasoning, attachments_json, token_usage, cost_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(normalized_conversation_id)
    .bind(&role)
    .bind(&content)
    .bind(reasoning)
    .bind(attachments_json)
    .bind(token_usage)
    .bind(cost_json)
    .bind(now)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    // Auto-title whenever a placeholder title is still present.
    if role.eq_ignore_ascii_case("user") {
        let current_title: Option<String> =
            sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
                .bind(normalized_conversation_id)
                .fetch_optional(&pool)
                .await
                .map_err(|e| e.to_string())?;

        let should_update = current_title
            .as_deref()
            .map(|title| {
                let trimmed = title.trim();
                trimmed.is_empty() || trimmed == "New chat"
            })
            .unwrap_or(true);

        if should_update {
            let first_user_content: Option<String> = sqlx::query_scalar(
                "SELECT content FROM conversation_messages WHERE conversation_id = ? AND role = 'user' ORDER BY created_at ASC, id ASC LIMIT 1",
            )
            .bind(normalized_conversation_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;

            let new_title = first_user_content
                .as_deref()
                .map(memory::derive_title_from_message)
                .unwrap_or_else(|| memory::derive_title_from_message(&content));
            sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
                .bind(new_title)
                .bind(normalized_conversation_id)
                .execute(&pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // Touch conversation timestamp so list order reflects latest activity.
    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(normalized_conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    // Keep summary memory in sync after each append.
    memory::refresh_conversation_memory(&pool, normalized_conversation_id, now).await?;

    Ok(())
}

pub async fn replace_history(
    app: &AppHandle,
    conversation_id: &str,
    messages: Vec<HistoryMessage>,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized_conversation_id = conversation_id.trim();

    if !conversation_exists(&pool, normalized_conversation_id).await? {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    let current_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
            .bind(normalized_conversation_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    let replacement_snapshot = if messages
        .last()
        .map(|message| message.role.eq_ignore_ascii_case("user"))
        .unwrap_or(false)
    {
        let user_ordinal = messages
            .iter()
            .filter(|message| message.role.eq_ignore_ascii_case("user"))
            .count();
        if user_ordinal <= 1 {
            Some(Vec::new())
        } else {
            let snapshot = load_turn_snapshot(app, normalized_conversation_id)
                .await?
                .ok_or_else(|| {
                    format!(
                        "会话 {} 缺少 turn snapshot，无法从数据库可见历史安全重建，请新开对话",
                        normalized_conversation_id
                    )
                })?;
            Some(snapshot_before_user_ordinal(&snapshot, user_ordinal))
        }
    } else {
        None
    };

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
        .bind(normalized_conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM conversation_tool_logs WHERE conversation_id = ?")
        .bind(normalized_conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM conversation_memory WHERE conversation_id = ?")
        .bind(normalized_conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM conversation_compact_boundaries WHERE conversation_id = ?")
        .bind(normalized_conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM conversation_turn_snapshots WHERE conversation_id = ?")
        .bind(normalized_conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    for (index, message) in messages.iter().enumerate() {
        let created_at = now + index as i64;
        let reasoning = message
            .reasoning
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let attachments_json = message
            .attachments
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let cost_json = message
            .cost
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, reasoning, attachments_json, token_usage, cost_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(normalized_conversation_id)
        .bind(&message.role)
        .bind(&message.content)
        .bind(reasoning)
        .bind(attachments_json)
        .bind(message.token_usage)
        .bind(cost_json)
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    let should_update_title = current_title
        .as_deref()
        .map(|title| {
            let trimmed = title.trim();
            trimmed.is_empty() || trimmed == "New chat"
        })
        .unwrap_or(true);

    if should_update_title {
        let next_title = messages
            .iter()
            .find(|message| message.role.eq_ignore_ascii_case("user"))
            .map(|message| memory::derive_title_from_message(&message.content))
            .unwrap_or_default();

        sqlx::query("UPDATE conversations SET title = ?, updated_at = ? WHERE id = ?")
            .bind(next_title)
            .bind(now)
            .bind(normalized_conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(normalized_conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    if !messages.is_empty() {
        memory::refresh_conversation_memory(&pool, normalized_conversation_id, now).await?;
    }
    if let Some(snapshot) = replacement_snapshot {
        save_turn_snapshot(app, normalized_conversation_id, &snapshot).await?;
    }

    Ok(())
}

pub async fn load_conversation_tool_logs(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<HistoryToolExecution>, String> {
    let pool = get_pool_with_schema(app).await?;

    let rows = sqlx::query(
        "SELECT log_id, turn_id, tool_name, input_text, result_text, status, started_at, finished_at FROM conversation_tool_logs WHERE conversation_id = ? ORDER BY started_at ASC, log_id ASC",
    )
    .bind(conversation_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let items = rows
        .into_iter()
        .map(|row| HistoryToolExecution {
            id: row.get::<String, _>("log_id"),
            turn_id: row.get::<Option<String>, _>("turn_id"),
            tool_name: row.get::<String, _>("tool_name"),
            input: row.get::<String, _>("input_text"),
            result: row.get::<String, _>("result_text"),
            status: row.get::<String, _>("status"),
            started_at: row.get::<i64, _>("started_at"),
            finished_at: row.get::<Option<i64>, _>("finished_at"),
        })
        .collect::<Vec<_>>();

    Ok(items)
}

pub async fn upsert_conversation_tool_log(
    app: &AppHandle,
    conversation_id: &str,
    log: HistoryToolExecution,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized_conversation_id = conversation_id.trim();

    // Tool traces can arrive after conversation deletion; ignore stale persistence.
    if !conversation_exists(&pool, normalized_conversation_id).await? {
        return Ok(());
    }

    let log_id = log.id.trim();
    if log_id.is_empty() {
        return Err("tool log id is required".to_string());
    }

    let tool_name = log.tool_name.trim();
    if tool_name.is_empty() {
        return Err("tool_name is required".to_string());
    }

    let status = log.status.trim().to_ascii_lowercase();
    if !matches!(
        status.as_str(),
        "running" | "completed" | "error" | "cancelled"
    ) {
        return Err(format!("invalid tool status: {}", log.status));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let started_at = if log.started_at > 0 {
        log.started_at
    } else {
        now
    };
    let finished_at = log
        .finished_at
        .and_then(|ts| if ts > 0 { Some(ts) } else { None });

    sqlx::query(
        r#"
        INSERT INTO conversation_tool_logs (
            conversation_id,
            log_id,
            turn_id,
            tool_name,
            input_text,
            result_text,
            status,
            started_at,
            finished_at,
            updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(conversation_id, log_id) DO UPDATE SET
            turn_id = excluded.turn_id,
            tool_name = excluded.tool_name,
            input_text = excluded.input_text,
            result_text = excluded.result_text,
            status = excluded.status,
            started_at = excluded.started_at,
            finished_at = excluded.finished_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(normalized_conversation_id)
    .bind(log_id)
    .bind(
        log.turn_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
    )
    .bind(tool_name)
    .bind(log.input)
    .bind(log.result)
    .bind(status)
    .bind(started_at)
    .bind(finished_at)
    .bind(now)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

// Clear history data.
// - with conversation_id: clear the scoped conversation's persisted data
// - without conversation_id: clear all persisted history and conversation rows
pub async fn clear_history(app: &AppHandle, conversation_id: Option<String>) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    if let Some(id) = conversation_id {
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_tool_logs WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_memory WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_compact_boundaries WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_turn_snapshots WHERE conversation_id = ?")
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
        crate::command::rag::rag_remove_conversation_documents(app, &id).await?;
        crate::llm::services::shell_sessions::close_session(Some(&id)).await;
        let _ = crate::llm::services::user_terminal::stop_session(Some(&id));
    } else {
        sqlx::query("DELETE FROM conversation_messages")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_tool_logs")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_memory")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_compact_boundaries")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversation_turn_snapshots")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM messages")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE token_usage_log SET conversation_id = NULL")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::command::rag::rag_remove_all_conversation_documents(app).await?;
        crate::llm::services::shell_sessions::close_all_sessions().await;
        crate::llm::services::user_terminal::close_all_sessions();
    }

    Ok(())
}

// Delete one conversation and all dependent rows.
// Order matters to satisfy FK constraints in environments that enforce them.
pub async fn delete_conversation(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;

    sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_tool_logs WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_memory WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_compact_boundaries WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_turn_snapshots WHERE conversation_id = ?")
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

    crate::command::rag::rag_remove_conversation_documents(app, conversation_id).await?;
    crate::llm::services::shell_sessions::close_session(Some(conversation_id)).await;
    let _ = crate::llm::services::user_terminal::stop_session(Some(conversation_id));

    Ok(())
}

pub async fn save_turn_snapshot(
    app: &AppHandle,
    conversation_id: &str,
    messages: &[crate::llm::types::Message],
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let json = serde_json::to_string(messages).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR REPLACE INTO conversation_turn_snapshots (conversation_id, snapshot_json, updated_at) VALUES (?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(&json)
    .bind(now)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn load_turn_snapshot(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Option<Vec<crate::llm::types::Message>>, String> {
    let pool = get_pool_with_schema(app).await?;
    let row: Option<String> = sqlx::query_scalar(
        "SELECT snapshot_json FROM conversation_turn_snapshots WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        None => Ok(None),
        Some(json) => {
            let messages = serde_json::from_str::<Vec<crate::llm::types::Message>>(&json)
                .map_err(|e| e.to_string())?;
            Ok(Some(messages))
        }
    }
}
