//! 会话事件日志存储：session_events 表的读写。
//!
//! 表为 append-only：只追加与按会话删除，永不更新单行。
//! 会话内写入串行（live_turns 单回合锁保证），seq 由库内最大值派生。

use sqlx::Row;
use tauri::AppHandle;

use super::events::SessionEvent;

/// 一条已落库事件。
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub seq: i64,
    pub turn_id: Option<String>,
    pub event: SessionEvent,
    pub created_at: i64,
}

/// session_events 建表语句（由 history::ensure_schema 调用）。
pub const SESSION_EVENTS_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS session_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        conversation_id TEXT NOT NULL,
        turn_id TEXT,
        seq INTEGER NOT NULL,
        event_type TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        UNIQUE (conversation_id, seq)
    );
    CREATE INDEX IF NOT EXISTS idx_session_events_conv
        ON session_events(conversation_id, seq);
"#;

/// 追加一条事件，返回其 seq。单事务：取 max(seq)+1 后插入。
/// 失败仅由调用方记日志，不阻断主对话流程（事件日志是记录设施，不是关键路径）。
pub async fn append_event(
    app: &AppHandle,
    conversation_id: &str,
    turn_id: Option<&str>,
    event: &SessionEvent,
) -> Result<i64, String> {
    let pool = crate::llm::history::history_pool(app).await?;
    let payload_json = serde_json::to_string(event).map_err(|e| e.to_string())?;
    let event_type = event.type_name();
    let turn_id_owned = turn_id.map(str::to_string);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO session_events (conversation_id, turn_id, seq, event_type, payload_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(&turn_id_owned)
    .bind(next_seq)
    .bind(event_type)
    .bind(&payload_json)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(next_seq)
}

/// 批量追加事件（同一事务，保证 seq 连续）。
pub async fn append_events(
    app: &AppHandle,
    conversation_id: &str,
    turn_id: Option<&str>,
    events: &[SessionEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let pool = crate::llm::history::history_pool(app).await?;
    let turn_id_owned = turn_id.map(str::to_string);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let mut next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp_millis();
    for event in events {
        let payload_json = serde_json::to_string(event).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO session_events (conversation_id, turn_id, seq, event_type, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(&turn_id_owned)
        .bind(next_seq)
        .bind(event.type_name())
        .bind(&payload_json)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        next_seq += 1;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 按序读取会话全部事件。
pub async fn load_events(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<StoredEvent>, String> {
    let pool = crate::llm::history::history_pool(app).await?;
    let rows = sqlx::query(
        "SELECT seq, turn_id, payload_json, created_at FROM session_events
         WHERE conversation_id = ? ORDER BY seq ASC",
    )
    .bind(conversation_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let payload_json: String = row.get("payload_json");
        let event: SessionEvent = match serde_json::from_str(&payload_json) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    conversation_id = %conversation_id,
                    seq = row.get::<i64, _>("seq"),
                    "skip malformed session event"
                );
                continue;
            }
        };
        out.push(StoredEvent {
            seq: row.get("seq"),
            turn_id: row.get("turn_id"),
            event,
            created_at: row.get("created_at"),
        });
    }
    Ok(out)
}

/// 删除会话全部事件（删除/清空会话时调用）。
pub async fn delete_events(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let pool = crate::llm::history::history_pool(app).await?;
    sqlx::query("DELETE FROM session_events WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 富化最后一条助手消息事件的展示成本（前端回写 transcript/压缩记录/耗时等
/// 仅 UI 层的元数据；属元数据补全，不新增事件）。无助手消息时静默返回。
pub async fn update_last_assistant_message_cost(
    app: &AppHandle,
    conversation_id: &str,
    cost: &serde_json::Value,
) -> Result<(), String> {
    let pool = crate::llm::history::history_pool(app).await?;
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT seq, payload_json FROM session_events
         WHERE conversation_id = ? AND event_type = 'assistant_message'
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(conversation_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some((seq, payload_json)) = row else {
        return Ok(());
    };

    let mut payload: serde_json::Value =
        serde_json::from_str(&payload_json).map_err(|e| e.to_string())?;
    payload["cost"] = cost.clone();
    let next_json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    sqlx::query("UPDATE session_events SET payload_json = ? WHERE conversation_id = ? AND seq = ?")
        .bind(&next_json)
        .bind(conversation_id)
        .bind(seq)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
