use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::llm::commands::types::{ConversationHandover, ConversationMemory, HistoryMessage};

pub fn derive_title_from_message(content: &str) -> String {
    // 取首行文本作为标题候选。
    let first_line = content.lines().next().unwrap_or("").trim();
    // 首行为空时回退到全文裁剪。
    let source = if first_line.is_empty() {
        content.trim()
    } else {
        first_line
    };
    // 标题最大字符数。
    let max_chars = 24usize;
    // 构建截断后的标题。
    let mut out = String::new();
    // 逐字符截断，避免 UTF-8 字节切分问题。
    for ch in source.chars().take(max_chars) {
        out.push(ch);
    }
    // 原文超过上限时追加省略号。
    if source.chars().count() > max_chars {
        format!("{}...", out)
    } else if out.is_empty() {
        // 为空时给默认标题。
        "New chat".to_string()
    } else {
        // 返回截断后的标题。
        out
    }
}

fn normalize_inline(text: &str) -> String {
    // 压缩任意空白为单空格，得到单行文本。
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn build_memory_from_history(
    messages: &[HistoryMessage],
    updated_at: i64,
) -> Option<ConversationMemory> {
    // 无消息时无法构建 memory。
    if messages.is_empty() {
        return None;
    }

    // 用最近 6 条消息合成摘要片段。
    let summary_parts = messages
        .iter()
        // 从最新往回取。
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        // 再恢复为时间正序。
        .into_iter()
        .rev()
        .map(|m| {
            // 统一说话者标签。
            let speaker = if m.role.eq_ignore_ascii_case("user") {
                "User"
            } else {
                "Nova"
            };
            // 归一化消息内容。
            let content = normalize_inline(&m.content);
            // 单条摘要截断到 120 字符。
            format!(
                "{}: {}",
                speaker,
                content.chars().take(120).collect::<String>()
            )
        })
        .collect::<Vec<_>>();

    // 用分隔符拼装摘要并整体截断到 800 字符。
    let summary = summary_parts
        .join(" | ")
        .chars()
        .take(800)
        .collect::<String>();
    // 纯空白摘要视为无效。
    if summary.trim().is_empty() {
        return None;
    }

    // 构建去重后的关键事实列表。
    let mut key_facts = Vec::new();
    // 扫描最近 12 条消息（时间正序）。
    for msg in messages.iter().rev().take(12).rev() {
        // 按行拆分，提取可能的事实句。
        for line in msg.content.split('\n') {
            // 归一化行文本。
            let normalized = normalize_inline(line);
            // 过滤过短/过长行，降低噪声。
            if normalized.len() < 12 || normalized.len() > 120 {
                continue;
            }
            // 忽略大小写去重。
            if key_facts
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&normalized))
            {
                continue;
            }
            // 追加新事实。
            key_facts.push(normalized);
            // 达到上限后停止。
            if key_facts.len() >= 8 {
                break;
            }
        }
        // 达到上限后停止外层循环。
        if key_facts.len() >= 8 {
            break;
        }
    }

    // 返回构建好的会话记忆。
    Some(ConversationMemory {
        summary,
        key_facts,
        updated_at,
    })
}

pub async fn refresh_conversation_memory(
    pool: &SqlitePool,
    conversation_id: &str,
    updated_at: i64,
) -> Result<(), String> {
    // 拉取最近 24 条消息用于重建 memory。
    let rows = sqlx::query(
        "SELECT role, content, reasoning, attachments_json, token_usage, cost_json FROM conversation_messages WHERE conversation_id = ? ORDER BY created_at DESC, id DESC LIMIT 24",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 映射为 HistoryMessage 列表。
    let mut messages = rows
        .into_iter()
        .map(|row| HistoryMessage {
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
                // 尝试反序列化为 JSON 值。
                .and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        })
        .collect::<Vec<_>>();

    // 恢复为时间正序。
    messages.reverse();

    // 若能构建 memory 则写入/更新数据库。
    if let Some(memory) = build_memory_from_history(&messages, updated_at) {
        // 序列化 key_facts。
        let key_facts_json = serde_json::to_string(&memory.key_facts).map_err(|e| e.to_string())?;
        sqlx::query(
            r#"
            INSERT INTO conversation_memory (conversation_id, summary, key_facts_json, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(conversation_id)
            DO UPDATE SET summary=excluded.summary, key_facts_json=excluded.key_facts_json, updated_at=excluded.updated_at
            "#,
        )
        // 绑定 conversation_id。
        .bind(conversation_id)
        // 绑定 summary。
        .bind(memory.summary)
        // 绑定 key_facts_json。
        .bind(key_facts_json)
        // 绑定 updated_at。
        .bind(memory.updated_at)
        // 执行 upsert。
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub async fn get_conversation_memory_by_pool(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Option<ConversationMemory>, String> {
    // 查询单条 memory 记录。
    let row = sqlx::query(
        "SELECT summary, key_facts_json, updated_at FROM conversation_memory WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 无记录时返回 None。
    let Some(row) = row else {
        return Ok(None);
    };

    // 读取 key_facts 原始 JSON。
    let key_facts_raw = row.get::<String, _>("key_facts_json");
    // 反序列化 key_facts，失败时回落空数组。
    let key_facts = serde_json::from_str::<Vec<String>>(&key_facts_raw).unwrap_or_default();

    // 构建返回对象。
    Ok(Some(ConversationMemory {
        summary: row.get::<String, _>("summary"),
        key_facts,
        updated_at: row.get::<i64, _>("updated_at"),
    }))
}

pub async fn get_conversation_handover_by_pool(
    pool: &SqlitePool,
    conversation_id: &str,
    recent_limit: Option<i64>,
) -> Result<ConversationHandover, String> {
    // recent limit 默认 12，限制在 [1, 50]。
    let limit = recent_limit.unwrap_or(12).clamp(1, 50);

    // 读取会话元信息，不存在则报错。
    let meta_row = sqlx::query("SELECT title, updated_at FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Conversation '{}' not found", conversation_id))?;

    // 查询总消息数。
    let total_message_count: i64 =
        sqlx::query_scalar("SELECT COUNT(1) FROM conversation_messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;

    // 拉取最近 limit 条消息。
    let rows = sqlx::query(
        "SELECT role, content, reasoning, attachments_json, token_usage, cost_json FROM conversation_messages WHERE conversation_id = ? ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 转换为 HistoryMessage。
    let mut recent_messages = rows
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
        .collect::<Vec<_>>();
    // 恢复为时间正序。
    recent_messages.reverse();

    // 读取会话更新时间。
    let updated_at = meta_row.get::<i64, _>("updated_at");
    // 优先使用已有 memory，否则从 recent messages 回建。
    let memory = match get_conversation_memory_by_pool(pool, conversation_id).await? {
        Some(memory) => memory,
        None => build_memory_from_history(&recent_messages, updated_at).ok_or_else(|| {
            format!(
                "Failed to build conversation memory for '{}' from recent history",
                conversation_id
            )
        })?,
    };

    // 构建 handover 结果。
    Ok(ConversationHandover {
        conversation_id: conversation_id.to_string(),
        title: meta_row.get::<String, _>("title"),
        summary: memory.summary,
        key_facts: memory.key_facts,
        recent_messages,
        omitted_message_count: (total_message_count - limit).max(0),
        total_message_count,
        updated_at,
    })
}

pub async fn upsert_conversation_memory_by_pool(
    pool: &SqlitePool,
    conversation_id: &str,
    summary: &str,
    key_facts: &[String],
) -> Result<(), String> {
    // 记录当前更新时间（unix 秒）。
    let now = chrono::Utc::now().timestamp();
    // 序列化 key_facts。
    let key_facts_json = serde_json::to_string(key_facts).map_err(|e| e.to_string())?;

    // 执行 upsert 写入 conversation_memory。
    sqlx::query(
        r#"
        INSERT INTO conversation_memory (conversation_id, summary, key_facts_json, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(conversation_id)
        DO UPDATE SET summary=excluded.summary, key_facts_json=excluded.key_facts_json, updated_at=excluded.updated_at
        "#,
    )
    // 绑定 conversation_id。
    .bind(conversation_id)
    // 绑定 summary。
    .bind(summary)
    // 绑定 key_facts_json。
    .bind(key_facts_json)
    // 绑定 now。
    .bind(now)
    // 执行 SQL。
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
