//! 会话事件日志存储：session_events 表的读写。
//!
//! 表为 append-only：只追加与按会话删除，永不更新单行。
//! 会话内写入串行（live_turns 单回合锁保证），seq 由库内最大值派生。

use std::collections::HashSet;
use sqlx::{Row, SqlitePool};
use crate::core::Core;
use crate::agent::utils::error_event::emit_backend_warning;
use crate::provider::types::{Content, ContentBlock, Message, Role};

use super::events::SessionEvent;

/// 一条已落库事件。
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub seq: i64,
    pub turn_id: Option<String>,
    pub trace_id: Option<String>,
    pub event: SessionEvent,
    pub created_at: i64,
}

/// session_events 建表语句（由 history::run_migrations 调用）。
pub const SESSION_EVENTS_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS session_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        conversation_id TEXT NOT NULL,
        turn_id TEXT,
        seq INTEGER NOT NULL,
        event_type TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        trace_id TEXT,
        UNIQUE (conversation_id, seq)
    );
    CREATE INDEX IF NOT EXISTS idx_session_events_conv
        ON session_events(conversation_id, seq);
"#;

async fn insert_events_tx(
    pool: &SqlitePool,
    conversation_id: &str,
    turn_id: Option<&str>,
    trace_id: Option<&str>,
    events: &[SessionEvent],
) -> Result<i64, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let mut next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let turn_id_owned = turn_id.map(str::to_string);
    let now = chrono::Utc::now().timestamp_millis();
    let mut last_seq = next_seq;
    for event in events {
        let payload_json = serde_json::to_string(event).map_err(|e| e.to_string())?;
        let effective_trace = event
            .trace_id()
            .map(str::to_string)
            .or_else(|| trace_id.map(str::to_string));
        sqlx::query(
            "INSERT INTO session_events (conversation_id, turn_id, seq, event_type, payload_json, created_at, trace_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(&turn_id_owned)
        .bind(next_seq)
        .bind(event.type_name())
        .bind(&payload_json)
        .bind(now)
        .bind(&effective_trace)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        last_seq = next_seq;
        next_seq += 1;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(last_seq)
}

fn report_store_write_failure(core: &Core, conversation_id: &str, event_type: &str, error: &str) {
    emit_backend_warning(
        core,
        "agent.session.store",
        format!(
            "事件日志持久化失败（conversation={}, event={}）：{}",
            conversation_id, event_type, error
        ),
        Some("session_events.write"),
    );
}

/// 追加一条事件，返回其 seq。单事务：取 max(seq)+1 后插入。
/// 写失败时会发出明确 `backend-warning` 告警，再将错误返回调用方。
pub async fn append_event(
    core: &Core,
    conversation_id: &str,
    turn_id: Option<&str>,
    event: &SessionEvent,
) -> Result<i64, String> {
    let pool = match crate::agent::session::history::history_pool(core).await {
        Ok(pool) => pool,
        Err(err) => {
            report_store_write_failure(core, conversation_id, event.type_name(), &err);
            return Err(err);
        }
    };
    let trace_id = event
        .trace_id()
        .map(str::to_string)
        .or_else(|| core.active_trace_id(Some(conversation_id), turn_id));
    match insert_events_tx(
        &pool,
        conversation_id,
        turn_id,
        trace_id.as_deref(),
        std::slice::from_ref(event),
    )
    .await
    {
        Ok(seq) => Ok(seq),
        Err(err) => {
            report_store_write_failure(core, conversation_id, event.type_name(), &err);
            Err(err)
        }
    }
}

/// 批量追加事件（同一事务，保证 seq 连续）。
pub async fn append_events(
    core: &Core,
    conversation_id: &str,
    turn_id: Option<&str>,
    events: &[SessionEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let pool = match crate::agent::session::history::history_pool(core).await {
        Ok(pool) => pool,
        Err(err) => {
            report_store_write_failure(core, conversation_id, "batch", &err);
            return Err(err);
        }
    };
    let trace_id = core.active_trace_id(Some(conversation_id), turn_id);
    if let Err(err) =
        insert_events_tx(&pool, conversation_id, turn_id, trace_id.as_deref(), events).await
    {
        report_store_write_failure(core, conversation_id, "batch", &err);
        return Err(err);
    }
    Ok(())
}

/// 直接在已知连接池上读取会话事件（避免池初始化期重入 `OnceCell`）。
pub(crate) async fn load_events_on_pool(
    pool: &SqlitePool,
    core: Option<&Core>,
    conversation_id: &str,
) -> Result<Vec<StoredEvent>, String> {
    let rows = sqlx::query(
        "SELECT seq, turn_id, trace_id, payload_json, created_at FROM session_events
         WHERE conversation_id = ? ORDER BY seq ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let seq: i64 = row.get("seq");
        let payload_json: String = row.get("payload_json");
        let event: SessionEvent = match serde_json::from_str(&payload_json) {
            Ok(event) => event,
            Err(error) => {
                if let Some(core) = core {
                    emit_backend_warning(
                        core,
                        "agent.session.store",
                        format!(
                            "跳过损坏的会话事件（conversation={}, seq={}）：{}",
                            conversation_id, seq, error
                        ),
                        Some("session_events.decode"),
                    );
                } else {
                    tracing::warn!(
                        error = %error,
                        conversation_id = %conversation_id,
                        seq,
                        "skip malformed session event"
                    );
                }
                continue;
            }
        };
        let trace_id: Option<String> = row
            .try_get::<Option<String>, _>("trace_id")
            .ok()
            .flatten()
            .or_else(|| event.trace_id().map(str::to_string));
        out.push(StoredEvent {
            seq,
            turn_id: row.get("turn_id"),
            trace_id,
            event,
            created_at: row.get("created_at"),
        });
    }
    Ok(out)
}

/// 按序读取会话全部事件。
pub async fn load_events(
    core: &Core,
    conversation_id: &str,
) -> Result<Vec<StoredEvent>, String> {
    let pool = crate::agent::session::history::history_pool(core).await?;
    load_events_on_pool(&pool, Some(core), conversation_id).await
}

/// 启动/崩溃补偿扫描：检测无配对 `TurnEnd` 的未闭合 `TurnStart`，
/// 为中断回合补全悬空工具终态、上下文 `ToolResult` 配对块与 `TurnEnd(interrupted)`。
pub(crate) async fn recover_interrupted_turns_on_pool(
    pool: &SqlitePool,
    core: Option<&Core>,
) -> Result<usize, String> {
    let open_rows = sqlx::query(
        r#"
        SELECT s.conversation_id, s.turn_id, MIN(s.seq) AS start_seq
        FROM session_events s
        WHERE s.event_type = 'turn_start'
          AND s.turn_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1 FROM session_events e
              WHERE e.conversation_id = s.conversation_id
                AND e.event_type = 'turn_end'
                AND e.turn_id = s.turn_id
          )
        GROUP BY s.conversation_id, s.turn_id
        ORDER BY s.conversation_id ASC, start_seq ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if open_rows.is_empty() {
        return Ok(0);
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut recovered = 0usize;

    for row in open_rows {
        let conversation_id: String = row.get("conversation_id");
        let turn_id: String = row.get("turn_id");
        let start_seq: i64 = row.get("start_seq");

        let events = load_events_on_pool(pool, core, &conversation_id).await?;
        let turn_trace_id = events.iter().find_map(|stored| match &stored.event {
            SessionEvent::TurnStart {
                turn_id: tid,
                trace_id,
            } if tid == &turn_id => trace_id.clone().or_else(|| stored.trace_id.clone()),
            _ => None,
        });

        let turn_slice: Vec<&StoredEvent> = events
            .iter()
            .filter(|stored| stored.seq >= start_seq)
            .take_while(|stored| match &stored.event {
                SessionEvent::TurnStart { turn_id: tid, .. } => tid == &turn_id,
                _ => true,
            })
            .filter(|stored| {
                stored
                    .turn_id
                    .as_deref()
                    .map(|tid| tid == turn_id)
                    .unwrap_or(true)
            })
            .collect();

        let mut finished_calls: HashSet<String> = HashSet::new();
        let mut dangling_calls: Vec<(String, String, i64)> = Vec::new();
        let mut assistant_tool_uses: Vec<String> = Vec::new();
        let mut resolved_tool_results: HashSet<String> = HashSet::new();

        for stored in &turn_slice {
            match &stored.event {
                SessionEvent::ToolCall {
                    call_id, tool_name, ..
                } => {
                    dangling_calls.push((call_id.clone(), tool_name.clone(), stored.created_at));
                }
                SessionEvent::ToolResult { call_id, .. } => {
                    finished_calls.insert(call_id.clone());
                }
                SessionEvent::AssistantMessage { message, .. } => {
                    if let Content::Blocks(blocks) = &message.content {
                        for block in blocks {
                            if let ContentBlock::ToolUse { id, .. } = block {
                                assistant_tool_uses.push(id.clone());
                            }
                        }
                    }
                }
                SessionEvent::ContextMessage { message } | SessionEvent::UserMessage { message, .. } => {
                    if let Content::Blocks(blocks) = &message.content {
                        for block in blocks {
                            if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                                resolved_tool_results.insert(tool_use_id.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let mut repair_events: Vec<SessionEvent> = Vec::new();
        for (call_id, tool_name, started_at) in dangling_calls {
            if finished_calls.contains(&call_id) {
                continue;
            }
            finished_calls.insert(call_id.clone());
            repair_events.push(SessionEvent::ToolResult {
                call_id,
                tool_name,
                output: "[Interrupted: process exited before tool completed]".to_string(),
                is_error: true,
                started_at,
                finished_at: now_ms,
                trace_id: turn_trace_id.clone(),
            });
        }

        let unresolved_tool_uses: Vec<ContentBlock> = assistant_tool_uses
            .into_iter()
            .filter(|id| !resolved_tool_results.contains(id))
            .map(|id| ContentBlock::ToolResult {
                tool_use_id: id,
                is_error: true,
                content: vec![ContentBlock::Text {
                    text: "[Interrupted: session terminated before tool finished]".to_string(),
                }],
            })
            .collect();

        if !unresolved_tool_uses.is_empty() {
            repair_events.push(SessionEvent::ContextMessage {
                message: Message {
                    role: Role::User,
                    content: Content::Blocks(unresolved_tool_uses),
                },
            });
        }

        repair_events.push(SessionEvent::TurnEnd {
            turn_id: turn_id.clone(),
            stop_reason: Some("interrupted".to_string()),
        });

        insert_events_tx(
            pool,
            &conversation_id,
            Some(&turn_id),
            turn_trace_id.as_deref(),
            &repair_events,
        )
        .await?;
        recovered += 1;
    }

    Ok(recovered)
}

/// 删除会话全部事件（删除/清空会话时调用）。
pub async fn delete_events(core: &Core, conversation_id: &str) -> Result<(), String> {
    let pool = crate::agent::session::history::history_pool(core).await?;
    sqlx::query("DELETE FROM session_events WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
