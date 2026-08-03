use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::llm::commands::types::{CompactBoundary, HistoryMessage, ResumeContext};

pub async fn get_conversation_resume_context_by_pool(
    pool: &SqlitePool,
    conversation_id: &str,
    boundary: CompactBoundary,
) -> Result<ResumeContext, String> {
    // 查询 compact 边界之后（含边界时刻）的消息。
    let rows = sqlx::query(
        r#"
        SELECT role, content, reasoning, attachments_json, token_usage, cost_json
        FROM conversation_messages
        WHERE conversation_id = ? AND created_at >= ?
        ORDER BY created_at ASC, id ASC
        "#,
    )
    // 绑定 conversation_id。
    .bind(conversation_id)
    // 绑定边界时间戳。
    .bind(boundary.created_at)
    // 拉取全部匹配记录。
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 将数据库行映射为 HistoryMessage 列表。
    let messages_since_boundary = rows
        .into_iter()
        .map(|row| HistoryMessage {
            id: None,
            // 读取 role。
            role: row.get::<String, _>("role"),
            // 读取 content。
            content: row.get::<String, _>("content"),
            reasoning: row.get::<Option<String>, _>("reasoning"),
            // 读取 attachments_json。
            attachments: row
                .get::<Option<String>, _>("attachments_json")
                .and_then(|s| serde_json::from_str(&s).ok()),
            // 读取 token_usage。
            token_usage: row.get::<Option<i64>, _>("token_usage"),
            cost: row
                // 读取 cost_json。
                .get::<Option<String>, _>("cost_json")
                // 解析为 JSON 值。
                .and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        })
        .collect::<Vec<_>>();

    // 返回 resume 上下文。
    Ok(ResumeContext {
        // 原样回传边界信息。
        boundary,
        // 边界之后消息列表。
        messages_since_boundary,
    })
}
