use sqlx::{Row, SqlitePool};

use crate::llm::commands::types::{CompactBoundary, CompactContext, ConversationHandover};

pub fn estimate_tokens(text: &str) -> i64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    // 按 tokenizer 家族分流估算，与 command::settings::estimate_text_tokens 保持一致。
    let mut tokens: f64 = 0.0;
    for ch in trimmed.chars() {
        let cp = ch as u32;
        if (ch as u8 as char).is_ascii_whitespace() && cp < 0x80 {
            tokens += 0.0;
        } else if cp >= 0x2E80 {
            tokens += 1.5;
        } else if cp >= 0x0080 {
            tokens += 1.0;
        } else {
            tokens += 0.25;
        }
    }
    tokens.ceil() as i64
}

pub fn build_compact_context(
    conversation_id: String,
    handover: ConversationHandover,
    token_budget: Option<i64>,
    recent_limit: Option<i64>,
) -> CompactContext {
    // recent_limit 默认 8，并限制在 [4, 24]。
    let recent_limit = recent_limit.unwrap_or(8).clamp(4, 24);
    // token_budget 默认 1600，并限制在 [400, 6000]。
    let token_budget = token_budget.unwrap_or(1600).clamp(400, 6000);

    // 从最近消息中抽取窗口，并恢复时间顺序拼装为文本。
    let recent_section = handover
        // 遍历最近消息。
        .recent_messages
        .iter()
        // 从末尾开始取最新消息。
        .rev()
        // 取 recent_limit 条。
        .take(recent_limit as usize)
        // 收集临时向量。
        .collect::<Vec<_>>()
        // 转回拥有所有权迭代器。
        .into_iter()
        // 再次反转以恢复旧->新顺序。
        .rev()
        .map(|m| {
            // 统一说话者标签。
            let speaker = if m.role.eq_ignore_ascii_case("user") {
                "User"
            } else {
                "Nova"
            };
            // 生成单条展示文本。
            format!("{}: {}", speaker, m.content.trim())
        })
        // 汇总文本行。
        .collect::<Vec<_>>()
        // 用空行分隔消息。
        .join("\n\n");

    // 构建 key facts 段落。
    let facts = if handover.key_facts.is_empty() {
        // 无 key facts 时返回空串。
        String::new()
    } else {
        handover
            .key_facts
            .iter()
            // 每条 key fact 加上 markdown 列表前缀。
            .map(|fact| format!("- {}", fact))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // 组装 compact 基础正文。
    let mut context_text = format!(
        "[Compact context]\nConversation: {}\nSummary: {}\n{}{}",
        handover.title,
        handover.summary,
        if facts.is_empty() { "" } else { "Key facts:\n" },
        facts
    );

    // 仅在 recent section 非空时追加最近消息段。
    if !recent_section.trim().is_empty() {
        context_text.push_str("\n\nRecent messages:\n");
        context_text.push_str(&recent_section);
    }

    // 估算当前全文 token。
    let estimated_tokens = estimate_tokens(&context_text);
    // 超预算则按字符粗略截断。
    let final_text = if estimated_tokens > token_budget {
        context_text
            .chars()
            // 近似按 token_budget*4 截断字符。
            .take((token_budget * 4) as usize)
            .collect::<String>()
    } else {
        // 未超预算时直接保留全文。
        context_text
    };

    // 返回 compact 上下文结构。
    CompactContext {
        conversation_id,
        // 存储最终文本副本。
        context_text: final_text.clone(),
        recent_limit,
        omitted_message_count: handover.omitted_message_count,
        total_message_count: handover.total_message_count,
        // 对最终文本重新估算 token。
        estimated_tokens: estimate_tokens(&final_text),
        updated_at: handover.updated_at,
    }
}

pub async fn record_compact_boundary_by_pool(
    pool: &SqlitePool,
    compact: &CompactContext,
    summary: &str,
    key_facts: &[String],
) -> Result<CompactBoundary, String> {
    // 记录创建时间戳（unix 秒）。
    let created_at = chrono::Utc::now().timestamp();
    // key_facts 序列化为 JSON 字符串存库。
    let key_facts_json = serde_json::to_string(key_facts).map_err(|e| e.to_string())?;

    // 插入 compact boundary 记录。
    let result = sqlx::query(
        r#"
        INSERT INTO conversation_compact_boundaries (
            conversation_id, context_text, summary, key_facts_json, recent_limit,
            omitted_message_count, total_message_count, estimated_tokens, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    // 绑定 conversation_id。
    .bind(&compact.conversation_id)
    // 绑定 compact 文本。
    .bind(&compact.context_text)
    // 绑定 summary。
    .bind(summary)
    // 绑定 key_facts JSON。
    .bind(&key_facts_json)
    // 绑定 recent_limit。
    .bind(compact.recent_limit)
    // 绑定 omitted_message_count。
    .bind(compact.omitted_message_count)
    // 绑定 total_message_count。
    .bind(compact.total_message_count)
    // 绑定 estimated_tokens。
    .bind(compact.estimated_tokens)
    // 绑定 created_at。
    .bind(created_at)
    // 执行插入。
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 返回内存态边界对象。
    Ok(CompactBoundary {
        // 使用数据库自增 ID。
        id: result.last_insert_rowid(),
        // 克隆 compact 字段到返回值。
        conversation_id: compact.conversation_id.clone(),
        context_text: compact.context_text.clone(),
        summary: summary.to_string(),
        // 复制 key_facts 切片为 Vec。
        key_facts: key_facts.to_vec(),
        recent_limit: compact.recent_limit,
        omitted_message_count: compact.omitted_message_count,
        total_message_count: compact.total_message_count,
        estimated_tokens: compact.estimated_tokens,
        created_at,
    })
}

pub async fn get_latest_compact_boundary_by_pool(
    pool: &SqlitePool,
    conversation_id: &str,
) -> Result<Option<CompactBoundary>, String> {
    // 查询指定会话最新的一条 compact boundary。
    let row = sqlx::query(
        r#"
        SELECT id, conversation_id, context_text, summary, key_facts_json,
               recent_limit, omitted_message_count, total_message_count,
               estimated_tokens, created_at
        FROM conversation_compact_boundaries
        WHERE conversation_id = ?
        ORDER BY created_at DESC, id DESC
        LIMIT 1
        "#,
    )
    // 绑定会话 ID。
    .bind(conversation_id)
    // 允许无记录返回 None。
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 没有结果时直接返回 None。
    let Some(row) = row else {
        return Ok(None);
    };

    // 读取 key_facts 原始 JSON 字符串。
    let key_facts_raw = row.get::<String, _>("key_facts_json");
    // 反序列化 key_facts，失败时回落空数组。
    let key_facts = serde_json::from_str::<Vec<String>>(&key_facts_raw).unwrap_or_default();

    // 构造返回对象。
    Ok(Some(CompactBoundary {
        id: row.get::<i64, _>("id"),
        conversation_id: row.get::<String, _>("conversation_id"),
        context_text: row.get::<String, _>("context_text"),
        summary: row.get::<String, _>("summary"),
        key_facts,
        recent_limit: row.get::<i64, _>("recent_limit"),
        omitted_message_count: row.get::<i64, _>("omitted_message_count"),
        total_message_count: row.get::<i64, _>("total_message_count"),
        estimated_tokens: row.get::<i64, _>("estimated_tokens"),
        created_at: row.get::<i64, _>("created_at"),
    }))
}