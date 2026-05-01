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

pub async fn delete_global_memory(
    app: &AppHandle,
    id: i64,
) -> Result<bool, String> {
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

fn resolved_conversation_title(current_title: &str, first_user_message: Option<&str>) -> String {
    let trimmed = current_title.trim();
    if !trimmed.is_empty() && trimmed != "New chat" {
        return trimmed.to_string();
    }

    first_user_message
        .map(memory::derive_title_from_message)
        .unwrap_or_else(|| trimmed.to_string())
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

    let rows = sqlx::query(
        r#"
        SELECT
            c.id,
            c.title,
            c.updated_at,
            (
                SELECT m.content
                FROM conversation_messages m
                WHERE m.conversation_id = c.id AND m.role = 'user'
                ORDER BY m.created_at ASC, m.id ASC
                LIMIT 1
            ) AS first_user_content
        FROM conversations c
        ORDER BY c.updated_at DESC
        "#,
    )
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

    let items = rows
        .into_iter()
        .map(|row| ConversationMeta {
            id: row.get::<String, _>("id"),
            title: resolved_conversation_title(
                &row.get::<String, _>("title"),
                row.get::<Option<String>, _>("first_user_content").as_deref(),
            ),
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

    // Auto-title whenever a placeholder title is still present.
    if role.eq_ignore_ascii_case("user") {
        let current_title: Option<String> = sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
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

    if role.eq_ignore_ascii_case("user") {
        if let Err(error) =
            crate::llm::services::memory_dir::remember_from_user_message(app, &content).await
        {
            crate::llm::utils::error_event::emit_backend_error(
                app,
                "memory.auto_remember",
                format!("全局记忆自动写入失败，本条消息不会被记忆模块索引：{}", error),
                Some("remember_from_user_message"),
            );
        }
    }

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
