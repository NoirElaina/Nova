mod state;
mod summary;

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::llm::types::{Content, ContentBlock, Message, Role};
use crate::llm::utils::token_counter;

// 策略阈值：完全按 token 比例触发。
// 不再使用消息条数或工具结果字符数等硬编码阈值，也不使用绝对值 buffer，
// 避免小上下文窗口模型触发"每轮必压缩"死结。
// Micro: 80% 窗口时做本地工具结果截断（不调用模型）
// Full: 90% 窗口时做模型摘要压缩

// 截断值：在 tool_result 里保持头尾信息, 避免 payload 过长。
// 旧值 1200 会截断 Read/Grep 返回的代码片段，agent 只看到片段头部+尾部，
// 丢失中间实现细节，被迫重新 Read 同一文件。提到 8000 可完整保留
// 约 200 行代码片段（典型文件大小），与 200K 上下文窗口相比成本可接受。
const TOOL_RESULT_TEXT_TRUNCATE_LIMIT: usize = 8000;

// JSON 压缩上限，避免深层数组/对象导致多次迭代爆炸。
const TOOL_RESULT_JSON_MAX_DEPTH: usize = 3;
const TOOL_RESULT_JSON_MAX_ITEMS: usize = 12;
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

// 文件内容类工具：其 ToolResult 包含文件内容/搜索结果，context editing 时不应被占位符替换。
// 清除后 agent 被迫重新 Read/Grep 同一文件，浪费工具调用轮次和输出 token。
// 这些结果由 micro-compact 的截断逻辑（更高阈值）处理，而不是被整段替换为占位符。
const PROTECTED_FILE_CONTENT_TOOLS: &[&str] = &["Read", "Grep", "Glob"];

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

fn collect_clearable_tool_result_ids(messages: &[Message]) -> Vec<String> {
    // 先构建 tool_use_id -> tool_name 映射，用于识别文件内容类工具（Read/Grep/Glob）。
    // 这些工具的 ToolResult 包含文件内容/搜索结果，被占位符替换后 agent 被迫重新调用，
    // 浪费工具轮次和输出 token；交给 micro-compact 的截断逻辑处理即可。
    let mut tool_name_by_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for message in messages {
        let Content::Blocks(blocks) = &message.content else {
            continue;
        };
        for block in blocks {
            if let ContentBlock::ToolUse { id, name, .. } = block {
                tool_name_by_id.insert(id.clone(), name.clone());
            }
        }
    }

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

            // 跳过文件内容类工具：Read/Grep/Glob 的结果保留，由 micro-compact 截断处理。
            let is_protected = tool_name_by_id
                .get(tool_use_id)
                .map(|name| PROTECTED_FILE_CONTENT_TOOLS.iter().any(|p| p == name))
                .unwrap_or(false);
            if is_protected {
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
    let original_estimated_tokens = token_counter::count_messages(messages);
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

    let edited_estimated_tokens = token_counter::count_messages(&edited_messages);
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

// 根据消息数量、估算 token 数和是否存在超大工具结果文本来决定压缩策略。
fn decide_compact_strategy(messages: &[Message], window_tokens: i64) -> CompactDecision {
    // 估算消息总体 token，纯粹基于 token 用量决定压缩等级。
    // 不使用消息条数或工具结果字符数等辅助条件。
    let estimated_tokens = token_counter::count_messages(messages);

    // Micro: 80% 窗口触发本地工具结果截断（不调用模型）
    // Full: 90% 窗口触发模型摘要压缩
    let micro_token_threshold = (window_tokens * 80) / 100;
    let full_token_threshold = (window_tokens * 90) / 100;

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

/// 最近一轮工具调用（最后一条含 ToolUse 的消息）的 tool_use id 集合。
/// 会话途中压缩时，这组结果即将在下一次请求被模型首次消费，
/// 截断它们会迫使模型重新 Read 同一文件，因此保留原文。
fn latest_tool_round_use_ids(messages: &[Message]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for message in messages.iter().rev() {
        let Content::Blocks(blocks) = &message.content else {
            continue;
        };
        let mut found = false;
        for block in blocks {
            if let ContentBlock::ToolUse { id, .. } = block {
                ids.insert(id.clone());
                found = true;
            }
        }
        if found {
            break;
        }
    }
    ids
}

fn apply_micro_compact(messages: &[Message]) -> Vec<Message> {
    apply_micro_compact_protected(messages, &HashSet::new())
}

// 对每条消息进行微压缩：仅压缩 ToolResult 内的长文本/JSON。
// protected_use_ids 对应的工具结果保留原文（会话途中 = 最近一轮未消费的结果）。
fn apply_micro_compact_protected(
    messages: &[Message],
    protected_use_ids: &HashSet<String>,
) -> Vec<Message> {
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
                                // 最近一轮未消费的工具结果保留原文
                                if protected_use_ids.contains(tool_use_id) {
                                    return block.clone();
                                }
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
    messages: &[Message],
    recent_limit: i64,
) -> Result<Option<Vec<Message>>, String> {
    let keep_count = recent_limit.clamp(6, 30) as usize;
    if messages.len() <= keep_count + 1 {
        return Ok(None);
    }

    let split_index = messages.len().saturating_sub(keep_count);
    if split_index == 0 {
        return Ok(None);
    }

    let messages_to_summarize = &messages[..split_index];
    let recent_messages = messages[split_index..].to_vec();
    let summary = summary::summarize_messages_for_compact(app, messages_to_summarize).await?;
    let compact_message = build_auto_compact_summary_message(&summary);

    let mut prepared = Vec::with_capacity(recent_messages.len() + 1);
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
        match try_model_driven_full_compact(app, messages, recent_limit).await {
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
    if messages.len() <= keep_recent {
        return messages.to_vec();
    }

    let mut start = messages.len().saturating_sub(keep_recent);
    while start > 0 && messages[start].role == Role::Assistant {
        start -= 1;
    }

    let mut prepared = messages[start..].to_vec();

    // 截断边界可能切在 ToolUse 与其 ToolResult 之间，保留区会残留
    // 失去配对的块——Anthropic/OpenAI 都强制配对完整，会直接 400。
    // 统一走孤儿块清理（与子代理截断共用同一份实现）。
    drop_orphan_tool_blocks(&mut prepared);
    prepared
}

/// 删除失去配对的 tool_use / tool_result 块（配对完整性由 API 强制）。
/// 主对话 reactive-compact 兜底截断与子代理 truncate_subagent_messages 共用。
pub fn drop_orphan_tool_blocks(messages: &mut Vec<Message>) {
    let mut use_ids: HashSet<String> = HashSet::new();
    let mut result_ids: HashSet<String> = HashSet::new();
    for message in messages.iter() {
        if let Content::Blocks(blocks) = &message.content {
            for block in blocks {
                match block {
                    ContentBlock::ToolUse { id, .. } => {
                        use_ids.insert(id.clone());
                    }
                    ContentBlock::ToolResult { tool_use_id, .. } => {
                        result_ids.insert(tool_use_id.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    for message in messages.iter_mut() {
        if let Content::Blocks(blocks) = &mut message.content {
            blocks.retain(|block| match block {
                ContentBlock::ToolUse { id, .. } => result_ids.contains(id),
                ContentBlock::ToolResult { tool_use_id, .. } => use_ids.contains(tool_use_id),
                _ => true,
            });
        }
    }

    // 清空后的空块消息整个移除（空 content 也可能被拒）。
    messages.retain(|m| match &m.content {
        Content::Blocks(blocks) => !blocks.is_empty(),
        Content::Text(text) => !text.trim().is_empty(),
    });
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
    // C1：先做 Micro 预处理再走 Full/兜底。否则摘要请求携带全量
    // tool_result 文本，自己就可能超限；摘要重试只按条数丢最老消息，
    // 大结果在近期时重试无效，连败 3 次熔断后该会话永久降级为兜底截断。
    let micro_compacted = apply_micro_compact(messages);

    let force_full = apply_full_compact_with_limits(
        app,
        conversation_id,
        &micro_compacted,
        REACTIVE_FULL_COMPACT_RECENT_LIMIT,
    )
    .await;
    if !same_messages(&force_full, &micro_compacted) {
        return Some(force_full);
    }

    let truncated =
        truncate_oldest_messages_for_retry(&micro_compacted, REACTIVE_FALLBACK_KEEP_MESSAGES);
    if !same_messages(&truncated, &micro_compacted) {
        return Some(truncated);
    }

    // 消息太少无法截断、Full 又被熔断/跳过时，Micro 结果也是净收益：
    // 截断后的大工具结果可能恰好腾出足够空间让重试通过。
    if !same_messages(&micro_compacted, messages) {
        return Some(micro_compacted);
    }

    None
}

#[derive(Debug)]
pub struct MidTurnCompactOutcome {
    pub level: &'static str,
    pub applied: bool,
    pub tokens_before: i64,
    pub tokens_after: i64,
}

/// 会话途中（主循环每次请求前）压缩：与轮开始同一强度（C2）。
/// - ≥90% 窗口：Full（Micro + 模型摘要，带熔断；熔断/失败时退回 Micro 结果）
/// - ≥80% 窗口：Micro（本地截断 tool_result 长文本/JSON）
///
/// 直接修改传入的 messages（与轮开始压缩一致，结果进入 turn snapshot）。
/// 最近一轮尚未被模型消费的工具结果保留原文，避免迫使模型重新 Read。
pub async fn compact_messages_mid_turn(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &mut Vec<Message>,
    window_tokens: i64,
) -> MidTurnCompactOutcome {
    let decision = decide_compact_strategy(messages, window_tokens);
    let tokens_before = decision.estimated_tokens;

    let level = match decision.level {
        CompactLevel::None => "none",
        CompactLevel::Micro => {
            let protected = latest_tool_round_use_ids(messages);
            let compacted = apply_micro_compact_protected(messages, &protected);
            if same_messages(&compacted, messages) {
                "none"
            } else {
                *messages = compacted;
                "micro"
            }
        }
        CompactLevel::Full => {
            // 与轮开始 Full 相同：先 Micro 再模型摘要；
            // 熔断打开或摘要失败时 apply_full_compact 原样返回，退回 Micro 结果。
            let protected = latest_tool_round_use_ids(messages);
            let micro = apply_micro_compact_protected(messages, &protected);
            let full = apply_full_compact(app, conversation_id, &micro).await;
            if !same_messages(&full, &micro) {
                *messages = full;
                "full"
            } else if !same_messages(&micro, messages) {
                *messages = micro;
                "micro"
            } else {
                "none"
            }
        }
    };

    let tokens_after = if level == "none" {
        tokens_before
    } else {
        token_counter::count_messages(messages)
    };

    MidTurnCompactOutcome {
        level,
        applied: level != "none",
        tokens_before,
        tokens_after,
    }
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
    let settings = crate::command::settings::load_settings(app)?;
    let model = settings.active_provider_profile().model;
    let window_tokens = settings.context_window_for_model(&model) as i64;

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

// 手动压缩：把当前会话历史发给 AI 摘要，用摘要+最近几条消息替换数据库历史。
// replace_history 会清除关联的工具日志/记忆/边界记录（旧消息已不存在，关联数据无意义）。
const MANUAL_COMPACT_RECENT_LIMIT: usize = 4;
const MANUAL_COMPACT_MIN_MESSAGES: usize = 6;

#[derive(Debug, Serialize)]
pub struct ManualCompactOutcome {
    pub before_tokens: u32,
    pub after_tokens: u32,
    pub saved_tokens: u32,
    pub summary: String,
}

pub async fn manual_compact(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<ManualCompactOutcome, String> {
    let history_messages = crate::llm::history::load_history(app, conversation_id).await?;
    if history_messages.len() <= MANUAL_COMPACT_MIN_MESSAGES {
        return Err("消息太少，无需压缩".to_string());
    }

    let messages: Vec<Message> = history_messages
        .iter()
        .map(|h| Message {
            role: if h.role.eq_ignore_ascii_case("user") {
                Role::User
            } else {
                Role::Assistant
            },
            content: Content::Text(h.content.clone()),
        })
        .collect();

    let before_tokens = token_counter::count_messages(&messages) as u32;

    let split_index = messages
        .len()
        .saturating_sub(MANUAL_COMPACT_RECENT_LIMIT);
    let messages_to_summarize = &messages[..split_index];
    let recent_messages = &messages[split_index..];

    let summary = summary::summarize_messages_for_compact(app, messages_to_summarize).await?;
    let compact_message = build_auto_compact_summary_message(&summary);

    let mut new_messages = vec![compact_message];
    new_messages.extend(recent_messages.iter().cloned());

    let after_tokens = token_counter::count_messages(&new_messages) as u32;

    // 事件日志时代：UI 历史从事件流投影（压缩只影响模型上下文，不丢用户可见历史）；
    // 模型上下文重建由下方的 CompactBoundary 检查点承担，不再需要快照。

    // 事件日志压缩检查点：重建模型上下文时丢弃旧事件，以压缩后上下文为新起点。
    let boundary = crate::llm::session_log::SessionEvent::CompactBoundary {
        base_context: new_messages.clone(),
        summary: summary.clone(),
        level: "manual".to_string(),
        tokens_before: before_tokens,
        tokens_after: after_tokens,
    };
    if let Err(error) =
        crate::llm::session_log::append_event(app, conversation_id, None, &boundary).await
    {
        tracing::warn!(error = %error, conversation_id = %conversation_id, "manual compact boundary append failed");
    }

    Ok(ManualCompactOutcome {
        before_tokens,
        after_tokens,
        saved_tokens: before_tokens.saturating_sub(after_tokens),
        summary,
    })
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
