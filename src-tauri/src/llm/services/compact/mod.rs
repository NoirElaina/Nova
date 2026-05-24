mod state;
mod summary;

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::llm::commands::types::CompactContext;
use crate::llm::types::{Content, ContentBlock, Message, Role};

// 每条消息、块、工具使用/工具结果的静态开销。用于 token 估算近似，防止只依赖字符数导致低估。
const TOKEN_OVERHEAD_PER_MESSAGE: i64 = 6;
const TOKEN_OVERHEAD_PER_BLOCK: i64 = 3;
const TOKEN_OVERHEAD_TOOL_USE: i64 = 20;
const TOKEN_OVERHEAD_TOOL_RESULT: i64 = 14;
const TOKEN_OVERHEAD_IMAGE_INPUT: i64 = 512;

// 策略阈值：完全按 token 比例触发（对标 Claude Code 的 autoCompact 策略）。
// 不再使用消息条数或工具结果字符数等硬编码阈值。
// Micro: 80% 窗口时做本地工具结果截断（不调用模型）
// Full: (窗口 - BUFFER) 时做模型摘要压缩（对标 Claude Code 的 windowSize - 13k）
const FULL_COMPACT_BUFFER_TOKENS: i64 = 13_000;

// 截断值：在 tool_result 里保持头尾信息, 避免 payload 过长。
const TOOL_RESULT_TEXT_TRUNCATE_LIMIT: usize = 1200;

// JSON 压缩上限，避免深层数组/对象导致多次迭代爆炸。
const TOOL_RESULT_JSON_MAX_DEPTH: usize = 3;
const TOOL_RESULT_JSON_MAX_ITEMS: usize = 12;
const SESSION_RESTORE_MARKER: &str = "[Session Restore Context]";
const REACTIVE_FULL_COMPACT_RECENT_LIMIT: i64 = 6;
const REACTIVE_FALLBACK_KEEP_MESSAGES: usize = 8;
const AUTO_COMPACT_SUMMARY_PREFIX: &str = "[Auto Compact Summary]";
// CONTEXT_EDIT 触发阈值按 50% 窗口大小动态计算，不再使用硬编码常量。
const CONTEXT_EDIT_KEEP_RECENT_TOOL_PAIRS: usize = 3;
const CONTEXT_EDIT_CLEAR_AT_LEAST_PAIRS: usize = 1;
const CONTEXT_EDIT_CLEAR_TOOL_INPUTS: bool = false;
const CONTEXT_EDIT_TOOL_RESULT_PLACEHOLDER: &str =
    "[tool_result removed by context editing to save prompt space]";
const CONTEXT_EDIT_TOOL_INPUT_PLACEHOLDER: &str = "[tool_use input removed by context editing]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactLevel {
    None,
    Micro,
    Full,
}

#[derive(Debug, Clone, Copy)]
struct CompactDecision {
    level: CompactLevel,
    estimated_tokens: i64,
}

pub struct CompactionOutcome {
    pub messages: Vec<Message>,
    pub estimated_tokens: i64,
    pub level: &'static str,
}

impl CompactionOutcome {
    pub fn did_compact(&self) -> bool {
        self.level != "none"
    }
}

#[derive(Debug)]
pub struct ToolResultContextEditingOutcome {
    pub messages: Vec<Message>,
    pub applied: bool,
    pub original_estimated_tokens: i64,
    pub edited_estimated_tokens: i64,
    pub cleared_tool_pairs: usize,
}

// 判断字符是否属于中日韩Unicode块。此处通过字节范围直接判断，避免调用 heavy regex。
fn is_cjk_char(ch: char) -> bool {
    let cp = ch as u32;
    // cp: Unicode code point of the character，用于范围匹配判断是否为 CJK 字符块。
    // 通过匹配 Unicode 范围判断，避免使用复杂或重量级的正则库。
    matches!(cp, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

// 估算纯文本片段的 token 数量。
// 规则：
// - CJK 每字符计 1 token
// - ASCII 字母数字 视为平均 4 个字符 1 token
// - 标点/符号按 2 字符 1 token
// - 空白按 16 字符 1 token
fn estimate_text_tokens(text: &str) -> i64 {
    // 统计各类字符数量，用于按经验规则估算 token 数。
    // cjk: 中日韩字符计数（每字符 ~1 token）
    let mut cjk = 0_i64;
    // latin_or_digit: ASCII 字母或数字计数，约 4 字符 = 1 token
    let mut latin_or_digit = 0_i64;
    // punctuation_or_symbol: 标点或符号计数，约 2 字符 = 1 token
    let mut punctuation_or_symbol = 0_i64;
    // whitespace: 空白字符计数，按更低权重合并
    let mut whitespace = 0_i64;

    for ch in text.chars() {
        // ch: 当前遍历到的字符
        if ch.is_whitespace() {
            // 空白字符（空格、换行等）按更低权重计算
            whitespace += 1;
        } else if is_cjk_char(ch) {
            // 中日韩字符每个近似 1 token
            cjk += 1;
        } else if ch.is_ascii_alphanumeric() {
            // ASCII 字母数字日常出现频率高：4 字符约 1 token
            latin_or_digit += 1;
        } else {
            // 标点/符号按 2 字符约 1 token
            punctuation_or_symbol += 1;
        }
    }

    // 最终汇总：按经验系数合并各类计数，使用上取整技巧避免低估
    // cjk + ceil(latin_or_digit/4) + ceil(punctuation_or_symbol/2) + ceil(whitespace/16)
    cjk + (latin_or_digit + 3) / 4 + (punctuation_or_symbol + 1) / 2 + (whitespace + 15) / 16
}

// 估算 JSON 数据结构的 token 大小，包含结构符号 + 键值 + 嵌套内容。
// 用于 tool_use / tool_result 中包含结构化 JSON 字符串时提供更加合理的估算。
fn estimate_json_tokens(value: &Value) -> i64 {
    match value {
        // 基本类型成本估算：Null/Bool/Number 使用低位成本
        Value::Null => 1,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        // 字符串先按文本估算再加上结构符成本
        Value::String(s) => estimate_text_tokens(s) + 1,
        Value::Array(items) => {
            // array: 计算头尾符号成本 + 每个项目的递归成本 + 项目分隔符成本
            2 + items.iter().map(estimate_json_tokens).sum::<i64>() + items.len() as i64
        }
        Value::Object(map) => {
            // object: 头尾成本 + 每个 key 的文本成本 + 对应 value 的递归成本 + 分隔符
            3 + map
                .iter()
                .map(|(k, v)| estimate_text_tokens(k) + estimate_json_tokens(v) + 2)
                .sum::<i64>()
        }
    }
}

// 估算一个 ContentBlock 的 token。对不同块类型使用差异化计算：
// - Text 按 text 字符内容估算
// - ToolUse 带 json 参数，需要额外估算 JSON 结构
// - ToolResult 递归估算嵌套块，并加上工具结果固定开销
fn estimate_block_tokens(block: &ContentBlock) -> i64 {
    match block {
        // 文本块：块开销 + 文本估算
        ContentBlock::Text { text } => TOKEN_OVERHEAD_PER_BLOCK + estimate_text_tokens(text),
        ContentBlock::Thinking { thinking, .. } => {
            TOKEN_OVERHEAD_PER_BLOCK + estimate_text_tokens(thinking)
        }
        // 图片块：采用固定近似开销，避免按 base64 字符长度严重高估。
        ContentBlock::Image { .. } => TOKEN_OVERHEAD_PER_BLOCK + TOKEN_OVERHEAD_IMAGE_INPUT,
        // 工具调用：块开销 + 固定工具使用开销 + 输入 JSON 的结构化估算
        ContentBlock::ToolUse { input, .. } => {
            TOKEN_OVERHEAD_PER_BLOCK + TOKEN_OVERHEAD_TOOL_USE + estimate_json_tokens(input)
        }
        // 工具结果：块开销 + 工具结果固定开销 + 嵌套内容的递归估算
        ContentBlock::ToolResult { content, .. } => {
            TOKEN_OVERHEAD_PER_BLOCK
                + TOKEN_OVERHEAD_TOOL_RESULT
                + content.iter().map(estimate_block_tokens).sum::<i64>()
        }
    }
}

// 更细颗粒度 token 估算：
// - 文本按 CJK/ASCII/符号分桶估算
// - 工具结构按 message/block/json 增加固定结构开销
fn estimate_message_tokens(messages: &[Message]) -> i64 {
    // 逐条消息估算：每条消息包含固定开销 + 消息体开销
    messages
        .iter()
        .map(|m| {
            // m: 当前消息引用
            let body = match &m.content {
                // 文本消息直接估算字符 token
                Content::Text(text) => estimate_text_tokens(text),
                // 块消息对每个块递归估算并求和
                Content::Blocks(blocks) => blocks.iter().map(estimate_block_tokens).sum::<i64>(),
            };
            // 每条消息的总估算 = 消息开销 + 内容开销
            TOKEN_OVERHEAD_PER_MESSAGE + body
        })
        .sum::<i64>()
}

fn message_has_session_restore_marker(message: &Message) -> bool {
    match &message.content {
        Content::Text(t) => t.contains(SESSION_RESTORE_MARKER),
        Content::Blocks(blocks) => blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                text.contains(SESSION_RESTORE_MARKER)
            } else {
                false
            }
        }),
    }
}

fn split_session_restore_message(messages: &[Message]) -> (Option<Message>, Vec<Message>) {
    let marker_index = messages.iter().position(message_has_session_restore_marker);
    let Some(marker_index) = marker_index else {
        return (None, messages.to_vec());
    };

    let mut rest = Vec::with_capacity(messages.len().saturating_sub(1));
    for (idx, msg) in messages.iter().enumerate() {
        if idx == marker_index {
            continue;
        }
        rest.push(msg.clone());
    }

    (Some(messages[marker_index].clone()), rest)
}

fn collect_tool_use_names(messages: &[Message]) -> HashMap<String, String> {
    let mut names = HashMap::new();

    for message in messages {
        let Content::Blocks(blocks) = &message.content else {
            continue;
        };

        for block in blocks {
            if let ContentBlock::ToolUse { id, name, .. } = block {
                names.insert(id.clone(), name.clone());
            }
        }
    }

    names
}

fn collect_clearable_tool_result_ids(messages: &[Message]) -> Vec<String> {
    let tool_names = collect_tool_use_names(messages);
    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    for message in messages {
        let Content::Blocks(blocks) = &message.content else {
            continue;
        };

        for block in blocks {
            let ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = block
            else {
                continue;
            };

            if !seen.insert(tool_use_id.clone()) {
                continue;
            }

            let excluded = tool_names
                .get(tool_use_id)
                .map(|_name| false)
                .unwrap_or(false);
            if excluded {
                continue;
            }

            let has_needs_user_input = content.iter().any(|inner| {
                let ContentBlock::Text { text } = inner else {
                    return false;
                };
                maybe_needs_user_input_payload(text)
            });
            if has_needs_user_input {
                continue;
            }

            ids.push(tool_use_id.clone());
        }
    }

    ids
}

pub fn apply_tool_result_context_editing(
    messages: &[Message],
    window_tokens: i64,
) -> ToolResultContextEditingOutcome {
    let original_estimated_tokens = estimate_message_tokens(messages);
    // 触发阈值 = 50% 窗口大小，比例与 decide_compact_strategy 的 Micro 阈值对齐。
    let context_edit_trigger = (window_tokens * 50) / 100;
    if original_estimated_tokens < context_edit_trigger {
        return ToolResultContextEditingOutcome {
            messages: messages.to_vec(),
            applied: false,
            original_estimated_tokens,
            edited_estimated_tokens: original_estimated_tokens,
            cleared_tool_pairs: 0,
        };
    }

    let clearable_ids = collect_clearable_tool_result_ids(messages);
    if clearable_ids.len() <= CONTEXT_EDIT_KEEP_RECENT_TOOL_PAIRS {
        return ToolResultContextEditingOutcome {
            messages: messages.to_vec(),
            applied: false,
            original_estimated_tokens,
            edited_estimated_tokens: original_estimated_tokens,
            cleared_tool_pairs: 0,
        };
    }

    let clear_count = clearable_ids
        .len()
        .saturating_sub(CONTEXT_EDIT_KEEP_RECENT_TOOL_PAIRS);
    if clear_count < CONTEXT_EDIT_CLEAR_AT_LEAST_PAIRS {
        return ToolResultContextEditingOutcome {
            messages: messages.to_vec(),
            applied: false,
            original_estimated_tokens,
            edited_estimated_tokens: original_estimated_tokens,
            cleared_tool_pairs: 0,
        };
    }

    let clear_ids: HashSet<String> = clearable_ids.into_iter().take(clear_count).collect();
    let cleared_tool_pairs = clear_ids.len();

    let edited_messages = messages
        .iter()
        .map(|message| {
            let content = match &message.content {
                Content::Text(text) => Content::Text(text.clone()),
                Content::Blocks(blocks) => Content::Blocks(
                    blocks
                        .iter()
                        .map(|block| match block {
                            ContentBlock::ToolUse { id, name, input } => {
                                if CONTEXT_EDIT_CLEAR_TOOL_INPUTS && clear_ids.contains(id) {
                                    ContentBlock::ToolUse {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input: json!({
                                            "_omitted": CONTEXT_EDIT_TOOL_INPUT_PLACEHOLDER
                                        }),
                                    }
                                } else {
                                    ContentBlock::ToolUse {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input: input.clone(),
                                    }
                                }
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                is_error,
                                content,
                            } => {
                                if clear_ids.contains(tool_use_id) {
                                    ContentBlock::ToolResult {
                                        tool_use_id: tool_use_id.clone(),
                                        is_error: *is_error,
                                        content: vec![ContentBlock::Text {
                                            text: CONTEXT_EDIT_TOOL_RESULT_PLACEHOLDER.to_string(),
                                        }],
                                    }
                                } else {
                                    ContentBlock::ToolResult {
                                        tool_use_id: tool_use_id.clone(),
                                        is_error: *is_error,
                                        content: content.clone(),
                                    }
                                }
                            }
                            _ => block.clone(),
                        })
                        .collect(),
                ),
            };

            Message {
                role: message.role.clone(),
                content,
            }
        })
        .collect::<Vec<_>>();

    let edited_estimated_tokens = estimate_message_tokens(&edited_messages);
    let applied = edited_estimated_tokens < original_estimated_tokens && cleared_tool_pairs > 0;

    ToolResultContextEditingOutcome {
        messages: if applied {
            edited_messages
        } else {
            messages.to_vec()
        },
        applied,
        original_estimated_tokens,
        edited_estimated_tokens: if applied {
            edited_estimated_tokens
        } else {
            original_estimated_tokens
        },
        cleared_tool_pairs: if applied { cleared_tool_pairs } else { 0 },
    }
}

fn build_auto_compact_summary_message(summary: &str) -> Message {
    Message {
        role: Role::User,
        content: Content::Text(format!(
            "{}\n{}",
            AUTO_COMPACT_SUMMARY_PREFIX,
            summary.trim()
        )),
    }
}

pub fn estimate_tokens_for_messages(messages: &[Message]) -> i64 {
    estimate_message_tokens(messages)
}

// 根据消息数量、估算 token 数和是否存在超大工具结果文本来决定压缩策略。
fn decide_compact_strategy(messages: &[Message], window_tokens: i64) -> CompactDecision {
    // 估算消息总体 token，纯粹基于 token 用量决定压缩等级。
    // 对标 Claude Code 策略：不使用消息条数或工具结果字符数等辅助条件。
    let estimated_tokens = estimate_message_tokens(messages);

    // Micro: 80% 窗口触发本地工具结果截断（不调用模型）
    // Full: (窗口 - 13k buffer) 触发模型摘要压缩
    let micro_token_threshold = (window_tokens * 80) / 100;
    let full_token_threshold = window_tokens - FULL_COMPACT_BUFFER_TOKENS;

    // 决策逻辑：优先判断 Full，再判断 Micro，否则 None
    let level = if estimated_tokens >= full_token_threshold {
        CompactLevel::Full
    } else if estimated_tokens >= micro_token_threshold {
        CompactLevel::Micro
    } else {
        CompactLevel::None
    };

    CompactDecision {
        level,
        estimated_tokens,
    }
}

fn maybe_needs_user_input_payload(text: &str) -> bool {
    // 尝试将 text 解析为 JSON 并检查 type 字段是否等于 "needs_user_input"
    serde_json::from_str::<serde_json::Value>(text)
        // 解析失败时返回 None
        .ok()
        // 若解析成功，尝试读取 v.get("type") 的字符串值并比较
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|s| s == "needs_user_input")
        })
        // 若任一步失败则返回 false
        .unwrap_or(false)
}

fn truncate_text_by_chars(text: &str, limit: usize) -> String {
    // len: 原文本字符长度
    let len = text.chars().count();
    // 若长度未超限则直接返回原文
    if len <= limit {
        return text.to_string();
    }

    // 将限制拆分为头部/尾部保留比例，留中间为省略信息
    let head_len = (limit * 60) / 100;
    let tail_len = (limit * 30) / 100;
    // omitted: 被截断省略的字符数（安全 saturating_sub 避免下溢）
    let omitted = len.saturating_sub(head_len + tail_len);

    // head: 文本前段
    let head: String = text.chars().take(head_len).collect();
    // tail: 文本后段，通过反向取并再反过来恢复原顺序
    let tail: String = text
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();

    // 最终格式包含 head、省略提示和 tail
    format!(
        "{}\n...[micro-compact truncated {} chars]...\n{}",
        head, omitted, tail
    )
}

fn compact_json_value(value: &Value, depth: usize) -> Value {
    // 深度限制：超过阈值直接返回占位字符串，避免递归展开过深导致成本爆炸
    if depth >= TOOL_RESULT_JSON_MAX_DEPTH {
        return Value::String("<truncated: max depth reached>".to_string());
    }

    match value {
        // 数组：只保留前 N 个元素并递归压缩每个元素
        Value::Array(items) => {
            let mut out: Vec<Value> = items
                .iter()
                .take(TOOL_RESULT_JSON_MAX_ITEMS)
                .map(|v| compact_json_value(v, depth + 1))
                .collect();
            // 若元素超出上限，记录被截断的数量
            if items.len() > TOOL_RESULT_JSON_MAX_ITEMS {
                out.push(serde_json::json!({
                    "_truncated_items": items.len() - TOOL_RESULT_JSON_MAX_ITEMS
                }));
            }
            Value::Array(out)
        }
        // 对象：按键排序后截断并递归压缩值
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();

            let mut out = serde_json::Map::new();
            for key in keys.into_iter().take(TOOL_RESULT_JSON_MAX_ITEMS) {
                if let Some(v) = map.get(key) {
                    out.insert(key.clone(), compact_json_value(v, depth + 1));
                }
            }
            // 若键数量超限，则在结果中记录被截断键的数量
            if map.len() > TOOL_RESULT_JSON_MAX_ITEMS {
                out.insert(
                    "_truncated_keys".to_string(),
                    Value::from((map.len() - TOOL_RESULT_JSON_MAX_ITEMS) as i64),
                );
            }
            Value::Object(out)
        }
        // 字符串：对长文本进行截断
        Value::String(s) => {
            Value::String(truncate_text_by_chars(s, TOOL_RESULT_TEXT_TRUNCATE_LIMIT))
        }
        // 其他原样返回
        _ => value.clone(),
    }
}

fn compact_tool_result_text(text: &str) -> String {
    // 交互类 payload 不能压缩，否则会破坏后续 ask-user 的语义。
    if maybe_needs_user_input_payload(text) {
        // 直接返回原文，不进行任何压缩或截断
        return text.to_string();
    }

    // 尝试解析为 JSON，并对 JSON 结构进行递归压缩与截断
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let compacted = compact_json_value(&value, 0);
        if let Ok(serialized) = serde_json::to_string(&compacted) {
            // 将压缩后的 JSON 序列化并截断为可接受长度
            return truncate_text_by_chars(&serialized, TOOL_RESULT_TEXT_TRUNCATE_LIMIT);
        }
    }

    // 非 JSON 或序列化失败时对原始文本按字符截断
    truncate_text_by_chars(text, TOOL_RESULT_TEXT_TRUNCATE_LIMIT)
}

fn apply_micro_compact(messages: &[Message]) -> Vec<Message> {
    // 对每条消息进行微压缩：仅压缩 ToolResult 内的长文本/JSON
    messages
        .iter()
        .map(|m| {
            // m: 当前消息
            let content = match &m.content {
                // 文本消息直接克隆
                Content::Text(text) => Content::Text(text.clone()),
                // 块消息：遍历每个块并只对 ToolResult 内部文本进行 compact
                Content::Blocks(blocks) => Content::Blocks(
                    blocks
                        .iter()
                        .map(|block| match block {
                            ContentBlock::ToolResult {
                                tool_use_id,
                                is_error,
                                content,
                            } => {
                                // compacted_content: 对 ToolResult 的内部块进行逐个压缩
                                let compacted_content = content
                                    .iter()
                                    .map(|inner| match inner {
                                        // 只压缩内部 Text 块的文本
                                        ContentBlock::Text { text } => ContentBlock::Text {
                                            text: compact_tool_result_text(text),
                                        },
                                        // 其他内部块保持不变
                                        _ => inner.clone(),
                                    })
                                    .collect();

                                // 返回压缩后的 ToolResult 块
                                ContentBlock::ToolResult {
                                    tool_use_id: tool_use_id.clone(),
                                    is_error: *is_error,
                                    content: compacted_content,
                                }
                            }
                            // 非 ToolResult 块直接克隆
                            _ => block.clone(),
                        })
                        .collect(),
                ),
            };

            // 构建并返回新的消息实体
            Message {
                role: m.role.clone(),
                content,
            }
        })
        .collect()
}

async fn apply_full_compact(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &[Message],
) -> Vec<Message> {
    apply_full_compact_with_limits(app, conversation_id, messages, 10).await
}

async fn try_model_driven_full_compact(
    app: &AppHandle,
    conversation_id: &str,
    messages: &[Message],
    recent_limit: i64,
) -> Result<Option<Vec<Message>>, String> {
    let (session_restore_message, messages_without_restore) =
        split_session_restore_message(messages);
    let keep_count = recent_limit.clamp(6, 30) as usize;
    if messages_without_restore.len() <= keep_count + 1 {
        return Ok(None);
    }

    let split_index = messages_without_restore.len().saturating_sub(keep_count);
    if split_index == 0 {
        return Ok(None);
    }

    let messages_to_summarize = &messages_without_restore[..split_index];
    let recent_messages = messages_without_restore[split_index..].to_vec();
    let summary = summary::summarize_messages_for_compact(app, messages_to_summarize).await?;
    let compact_message = build_auto_compact_summary_message(&summary);

    if let Ok(handover) = crate::command::history::get_conversation_handover(
        app.clone(),
        conversation_id.to_string(),
        Some(recent_limit),
    )
    .await
    {
        let compact_context = CompactContext {
            conversation_id: conversation_id.to_string(),
            context_text: match &compact_message.content {
                Content::Text(text) => text.clone(),
                Content::Blocks(_) => summary.clone(),
            },
            recent_limit,
            omitted_message_count: handover.omitted_message_count,
            total_message_count: handover.total_message_count,
            estimated_tokens: estimate_text_tokens(&summary),
            updated_at: handover.updated_at,
        };

        if let Err(error) = crate::command::history::record_compact_boundary(
            app.clone(),
            &compact_context,
            &summary,
            &Vec::new(),
        )
        .await
        {
            tracing::warn!(
                operation = "llm.services.compact.record_compact_boundary",
                conversation_id = %conversation_id,
                error = %error,
                "failed to persist compact boundary"
            );
        }
    }

    let mut prepared = Vec::with_capacity(
        recent_messages.len() + 1 + usize::from(session_restore_message.is_some()),
    );
    if let Some(restore) = session_restore_message {
        prepared.push(restore);
    }
    prepared.push(compact_message);
    prepared.extend(recent_messages);
    Ok(Some(prepared))
}

async fn apply_full_compact_with_limits(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &[Message],
    recent_limit: i64,
) -> Vec<Message> {
    // 若 conversation_id 为空或仅空白则不做 Full 压缩，直接返回原消息
    let Some(conversation_id) = conversation_id.filter(|id| !id.trim().is_empty()) else {
        return messages.to_vec();
    };

    if !state::is_auto_compact_circuit_open(Some(conversation_id)) {
        match try_model_driven_full_compact(app, conversation_id, messages, recent_limit).await {
            Ok(Some(compacted)) => {
                state::record_auto_compact_success(Some(conversation_id));
                return compacted;
            }
            Ok(None) => {}
            Err(error) => {
                let failures = state::record_auto_compact_failure(Some(conversation_id));
                tracing::warn!(
                    operation = "llm.services.compact.model_driven_auto_compact",
                    conversation_id = %conversation_id,
                    consecutive_failures = failures,
                    error = %error,
                    "model-driven auto compact failed"
                );
                crate::llm::utils::error_event::emit_backend_error(
                    app,
                    "compact.full_compact",
                    format!(
                        "会话上下文压缩失败（连续失败 {} 次），本次跳过压缩：{}",
                        failures, error
                    ),
                    Some("model_driven_compact"),
                );
                return messages.to_vec();
            }
        }
    } else {
        tracing::warn!(
            operation = "llm.services.compact.model_driven_auto_compact",
            conversation_id = %conversation_id,
            "model-driven auto compact skipped because circuit breaker is open"
        );
        return messages.to_vec();
    }
    messages.to_vec()
}

fn same_messages(left: &[Message], right: &[Message]) -> bool {
    serde_json::to_string(left).ok() == serde_json::to_string(right).ok()
}

fn truncate_oldest_messages_for_retry(messages: &[Message], keep_recent: usize) -> Vec<Message> {
    let (session_restore_message, messages_without_restore) =
        split_session_restore_message(messages);

    if messages_without_restore.len() <= keep_recent {
        return messages.to_vec();
    }

    let mut start = messages_without_restore.len().saturating_sub(keep_recent);
    while start > 0 && messages_without_restore[start].role == Role::Assistant {
        start -= 1;
    }

    let mut prepared = Vec::with_capacity(
        messages_without_restore.len().saturating_sub(start)
            + usize::from(session_restore_message.is_some()),
    );
    if let Some(restore) = session_restore_message {
        prepared.push(restore);
    }
    prepared.extend(messages_without_restore[start..].iter().cloned());
    prepared
}

pub fn is_prompt_too_long_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    [
        "prompt_too_long",
        "prompt too long",
        "context length",
        "context too long",
        "maximum context length",
        "context window",
        "too many tokens",
        "token limit exceeded",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub async fn reactive_compact_messages_for_retry(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &[Message],
) -> Option<Vec<Message>> {
    let force_full = apply_full_compact_with_limits(
        app,
        conversation_id,
        messages,
        REACTIVE_FULL_COMPACT_RECENT_LIMIT,
    )
    .await;
    if !same_messages(&force_full, messages) {
        return Some(force_full);
    }

    let truncated = truncate_oldest_messages_for_retry(messages, REACTIVE_FALLBACK_KEEP_MESSAGES);
    if !same_messages(&truncated, messages) {
        return Some(truncated);
    }

    None
}

// 入口：按层级执行 compact（纯压缩，不负责组装额外上下文）。
// - None: 不压缩
// - Micro: 仅本地清洗 tool_result（尤其长 JSON/长文本）
// - Full: 先做 Micro，再拼接 compact 历史上下文 + 最近窗口
pub async fn compact_messages_for_turn_with_report(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &[Message],
) -> Result<CompactionOutcome, String> {
    // 从 settings 读取当前模型的上下文窗口大小，用于动态计算压缩阈值。
    let model = crate::command::settings::get_settings(app.clone())?
        .active_provider_profile()
        .model;
    let window_tokens = crate::llm::utils::model_context::get_context_window_tokens(&model) as i64;

    // 决策并记录调试信息
    let decision = decide_compact_strategy(messages, window_tokens);
    // 根据决策执行对应的压缩流程
    let level = match decision.level {
        CompactLevel::None => "none",
        CompactLevel::Micro => "micro",
        CompactLevel::Full => "full",
    };

    let messages = match decision.level {
        CompactLevel::None => messages.to_vec(),
        CompactLevel::Micro => apply_micro_compact(messages),
        CompactLevel::Full => {
            // Full 先做 Micro 级别的局部压缩，再拼接远端 compact 上下文
            let micro_compacted = apply_micro_compact(messages);
            apply_full_compact(app, conversation_id, &micro_compacted).await
        }
    };

    Ok(CompactionOutcome {
        messages,
        estimated_tokens: decision.estimated_tokens,
        level,
    })
}

pub async fn compact_messages_for_turn(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &[Message],
) -> Result<Vec<Message>, String> {
    compact_messages_for_turn_with_report(app, conversation_id, messages)
        .await
        .map(|outcome| outcome.messages)
}

// 检查当前输出消息是否包含工具结果里标记为需要用户输入的 payload，
// 用于跑宏任务时暂停回合并向前端触发交互。
pub fn has_needs_user_input(messages: &[Message]) -> bool {
    // 遍历消息，判断任一 ToolResult 内是否包含需要用户输入的 payload
    messages.iter().any(|m| {
        // 使用 let-else 结构快速排除非 blocks 类型的消息
        let Content::Blocks(blocks) = &m.content else {
            return false;
        };

        // blocks: 消息内的块序列，查找任一 ToolResult
        blocks.iter().any(|b| {
            // 只关注 ToolResult 块
            let ContentBlock::ToolResult { content, .. } = b else {
                return false;
            };

            // 在 ToolResult 的内部块中查找 Text 块并解析其 JSON 字符串
            content.iter().any(|inner| {
                let ContentBlock::Text { text } = inner else {
                    return false;
                };

                // 解析 JSON 字符串，若 type=="needs_user_input" 则认为需要用户继续输入
                serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|v| {
                        v.get("type")
                            .and_then(|t| t.as_str())
                            .map(|s| s == "needs_user_input")
                    })
                    .unwrap_or(false)
            })
        })
    })
}
