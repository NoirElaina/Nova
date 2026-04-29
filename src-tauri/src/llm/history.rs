use serde_json::Value;
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::llm::commands::memory;
use crate::llm::commands::types::{
    ConversationMeta, GlobalMemoryEntry, HistoryMessage, HistoryToolExecution,
};

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
            updated_at INTEGER NOT NULL
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

        CREATE TABLE IF NOT EXISTS global_memory (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            kind TEXT NOT NULL,
            source TEXT NOT NULL,
            content_hash TEXT NOT NULL UNIQUE,
            hits INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_global_memory_updated_at ON global_memory(updated_at DESC);
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

fn normalize_global_memory_content(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_global_memory_kind(raw: Option<&str>) -> String {
    match raw.unwrap_or("fact").trim().to_ascii_lowercase().as_str() {
        "preference" => "preference".to_string(),
        "rule" => "rule".to_string(),
        _ => "fact".to_string(),
    }
}

fn normalize_global_memory_source(raw: Option<&str>) -> String {
    let normalized = raw.unwrap_or("assistant").trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "assistant".to_string()
    } else {
        normalized
    }
}

fn hash_global_memory_content(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn map_global_memory_row(row: sqlx::sqlite::SqliteRow) -> GlobalMemoryEntry {
    GlobalMemoryEntry {
        id: row.get::<i64, _>("id"),
        content: row.get::<String, _>("content"),
        kind: row.get::<String, _>("kind"),
        source: row.get::<String, _>("source"),
        hits: row.get::<i64, _>("hits"),
        created_at: row.get::<i64, _>("created_at"),
        updated_at: row.get::<i64, _>("updated_at"),
    }
}

pub async fn upsert_global_memory(
    app: &AppHandle,
    content: &str,
    kind: Option<&str>,
    source: Option<&str>,
) -> Result<GlobalMemoryEntry, String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized_content = normalize_global_memory_content(content);
    if normalized_content.is_empty() {
        return Err("global memory content is empty".to_string());
    }

    let kind = normalize_global_memory_kind(kind);
    let source = normalize_global_memory_source(source);
    let content_hash = hash_global_memory_content(&normalized_content);
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        r#"
        INSERT INTO global_memory (content, kind, source, content_hash, hits, created_at, updated_at)
        VALUES (?, ?, ?, ?, 1, ?, ?)
        ON CONFLICT(content_hash) DO UPDATE SET
            content = excluded.content,
            kind = excluded.kind,
            source = excluded.source,
            hits = global_memory.hits + 1,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&normalized_content)
    .bind(&kind)
    .bind(&source)
    .bind(&content_hash)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query(
        "SELECT id, content, kind, source, hits, created_at, updated_at FROM global_memory WHERE content_hash = ?",
    )
    .bind(&content_hash)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(map_global_memory_row(row))
}

pub async fn list_global_memory(
    app: &AppHandle,
    limit: Option<i64>,
) -> Result<Vec<GlobalMemoryEntry>, String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized_limit = limit.unwrap_or(12).clamp(1, 100);

    let rows = sqlx::query(
        "SELECT id, content, kind, source, hits, created_at, updated_at FROM global_memory ORDER BY updated_at DESC, id DESC LIMIT ?",
    )
    .bind(normalized_limit)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(map_global_memory_row).collect::<Vec<_>>())
}

pub async fn delete_global_memory(
    app: &AppHandle,
    id: i64,
) -> Result<bool, String> {
    let pool = get_pool_with_schema(app).await?;
    let result = sqlx::query("DELETE FROM global_memory WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

pub async fn clear_global_memory(app: &AppHandle) -> Result<i64, String> {
    let pool = get_pool_with_schema(app).await?;
    let result = sqlx::query("DELETE FROM global_memory")
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() as i64)
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

// Create a new conversation row with generated UUID and optional title.
pub async fn create_conversation(
    app: &AppHandle,
    title: Option<String>,
) -> Result<ConversationMeta, String> {
    let pool = get_pool_with_schema(app).await?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let conv_title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_default();

    sqlx::query("INSERT INTO conversations (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(&conv_title)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ConversationMeta {
        id,
        title: conv_title,
        updated_at: now,
    })
}

// List conversations ordered by latest update time.
pub async fn list_conversations(app: &AppHandle) -> Result<Vec<ConversationMeta>, String> {
    let pool = get_pool_with_schema(app).await?;

    let rows = sqlx::query("SELECT id, title, updated_at FROM conversations ORDER BY updated_at DESC")
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

    let items = rows
        .into_iter()
        .map(|row| ConversationMeta {
            id: row.get::<String, _>("id"),
            title: row.get::<String, _>("title"),
            updated_at: row.get::<i64, _>("updated_at"),
        })
        .collect();

    Ok(items)
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

    // Auto-title only when first user message arrives and current title is empty.
    if role.eq_ignore_ascii_case("user") {
        let user_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM conversation_messages WHERE conversation_id = ? AND role = 'user'",
        )
        .bind(normalized_conversation_id)
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;

        if user_count == 1 {
            let current_title: Option<String> =
                sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
                    .bind(normalized_conversation_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| e.to_string())?;

            let should_update = current_title
                .as_deref()
                .map(|title| title.trim().is_empty())
                .unwrap_or(true);

            if should_update {
                let new_title = memory::derive_title_from_message(&content);
                sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
                    .bind(new_title)
                    .bind(normalized_conversation_id)
                    .execute(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
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

pub async fn load_conversation_tool_logs(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<HistoryToolExecution>, String> {
    let pool = get_pool_with_schema(app).await?;

    let rows = sqlx::query(
        "SELECT log_id, tool_name, input_text, result_text, status, started_at, finished_at FROM conversation_tool_logs WHERE conversation_id = ? ORDER BY started_at ASC, log_id ASC",
    )
    .bind(conversation_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let items = rows
        .into_iter()
        .map(|row| HistoryToolExecution {
            id: row.get::<String, _>("log_id"),
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
    if !matches!(status.as_str(), "running" | "completed" | "error" | "cancelled") {
        return Err(format!("invalid tool status: {}", log.status));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let started_at = if log.started_at > 0 { log.started_at } else { now };
    let finished_at = log.finished_at.and_then(|ts| if ts > 0 { Some(ts) } else { None });

    sqlx::query(
        r#"
        INSERT INTO conversation_tool_logs (
            conversation_id,
            log_id,
            tool_name,
            input_text,
            result_text,
            status,
            started_at,
            finished_at,
            updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(conversation_id, log_id) DO UPDATE SET
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

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::command::rag::rag_remove_conversation_documents(app, &id)?;
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
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM messages")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::command::rag::rag_remove_all_conversation_documents(app)?;
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

    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    crate::command::rag::rag_remove_conversation_documents(app, conversation_id)?;

    Ok(())
}
