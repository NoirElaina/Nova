//! 事件流投影：从 session_events 渲染出三种视图——
//! 1. UI 聊天历史（HistoryMessage 列表，与旧接口返回结构一致）；
//! 2. 工具执行日志（HistoryToolExecution 列表）；
//! 3. 模型上下文重建（按序拼消息，遇 CompactBoundary 重置为其 base_context）。

use crate::llm::commands::types::{HistoryMessage, HistoryToolExecution};
use crate::llm::types::{Content, ContentBlock, Role};
use serde::Serialize;
use serde_json::Value;

use super::events::SessionEvent;
use super::store::StoredEvent;

/// 提取消息中的纯文本（Text 块拼接；跳过 tool/thinking 块）。
fn extract_text(message: &crate::llm::types::Message) -> String {
    content_text(&message.content)
}

/// 从 Content 提取纯文本（Text 块拼接；跳过 tool/thinking 块）。
pub fn content_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => {
            let mut out = String::new();
            for block in blocks {
                if let ContentBlock::Text { text } = block {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            out
        }
    }
}

/// 提取消息中的推理/思考文本（Thinking 块拼接）。
fn extract_reasoning(message: &crate::llm::types::Message) -> Option<String> {
    let Content::Blocks(blocks) = &message.content else {
        return None;
    };
    let mut out = String::new();
    for block in blocks {
        if let ContentBlock::Thinking { thinking, .. } = block {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(thinking);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 投影 UI 聊天历史：仅 UserMessage / AssistantMessage 进消息流，
/// id 用事件 seq（稳定唯一，前端作消息键使用）。
pub fn render_ui_history(events: &[StoredEvent]) -> Vec<HistoryMessage> {
    let mut out = Vec::new();
    for stored in events {
        match &stored.event {
            SessionEvent::UserMessage { message, attachments } => {
                out.push(HistoryMessage {
                    id: Some(stored.seq),
                    role: "user".to_string(),
                    content: extract_text(message),
                    reasoning: extract_reasoning(message),
                    attachments: attachments.clone(),
                    token_usage: None,
                    cost: None,
                });
            }
            SessionEvent::AssistantMessage { message, token_usage, cost } => {
                out.push(HistoryMessage {
                    id: Some(stored.seq),
                    role: "assistant".to_string(),
                    content: extract_text(message),
                    reasoning: extract_reasoning(message),
                    attachments: None,
                    token_usage: *token_usage,
                    cost: cost.clone(),
                });
            }
            _ => {}
        }
    }
    out
}

/// 投影工具执行日志：ToolCall 与 ToolResult 按 call_id 配对；
/// 只有 ToolCall 没有结果 = running。
pub fn render_tool_logs(events: &[StoredEvent]) -> Vec<HistoryToolExecution> {
    let mut logs: Vec<HistoryToolExecution> = Vec::new();
    for stored in events {
        match &stored.event {
            SessionEvent::ToolCall { call_id, tool_name, input, turn_id } => {
                logs.push(HistoryToolExecution {
                    id: call_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_name: tool_name.clone(),
                    input: input.clone(),
                    result: String::new(),
                    status: "running".to_string(),
                    started_at: stored.created_at,
                    finished_at: None,
                });
            }
            SessionEvent::ToolResult { call_id, output, is_error, finished_at, .. } => {
                if let Some(entry) = logs.iter_mut().find(|log| log.id == *call_id) {
                    entry.result = output.clone();
                    entry.status = if *is_error { "error" } else { "completed" }.to_string();
                    entry.finished_at = Some(*finished_at);
                }
            }
            _ => {}
        }
    }
    logs
}

/// 重建模型上下文：按序拼接消息事件，遇到 CompactBoundary 丢弃之前
/// 的全部内容并以其 base_context 为新起点。
pub fn reconstruct_model_context(events: &[StoredEvent]) -> Vec<crate::llm::types::Message> {
    let mut context: Vec<crate::llm::types::Message> = Vec::new();
    for stored in events {
        match &stored.event {
            SessionEvent::UserMessage { message, .. }
            | SessionEvent::AssistantMessage { message, .. }
            | SessionEvent::ContextMessage { message } => {
                context.push(message.clone());
            }
            SessionEvent::CompactBoundary { base_context, .. } => {
                context = base_context.clone();
            }
            _ => {}
        }
    }
    context
}

/// 事件流中是否存在消息内容（会话列表判空/首轮判断用）。
pub fn has_user_message(events: &[StoredEvent]) -> bool {
    events.iter().any(|stored| {
        matches!(
            &stored.event,
            SessionEvent::UserMessage { message, .. }
                if matches!(message.role, Role::User)
        )
    })
}

// ─────────────────────────────────────────────
// 聊天轨迹（调试面板用）：按回合分组的事件流投影。
// ─────────────────────────────────────────────

/// 单个回合的完整轨迹。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTrace {
    pub turn_id: String,
    /// 回合在事件流中的序号（排序/定位用）。
    pub seq: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub stop_reason: Option<String>,
    /// 用户输入原文（本回合首条用户消息）。
    pub user_input: Option<String>,
    /// 助手输出正文（多个 assistant 事件拼接）。
    pub assistant_text: String,
    /// 助手推理/思考文本拼接。
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallTrace>,
    /// 注入类上下文消息条数（hook/压缩注入等，不展示原文）。
    pub injected_context_count: usize,
    pub compactions: Vec<CompactionTrace>,
    pub token_usage: Option<i64>,
    pub cost: Option<Value>,
    /// 本回合发给模型 API 的 wire 级请求/响应（按顺序配对）。
    pub wire_calls: Vec<WireCallTrace>,
}

/// 单次模型 API 请求及其响应。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCallTrace {
    pub url: String,
    /// 请求报文完整 JSON（含 system prompt/tools/消息数组）。
    pub request: Value,
    /// 流结束后的完整回复文本。
    pub response_text: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

/// 回合内单次工具调用的轨迹。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallTrace {
    pub call_id: String,
    pub tool_name: String,
    pub input: String,
    pub output: Option<String>,
    pub is_error: bool,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

/// 回合内发生的压缩事件。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionTrace {
    pub level: String,
    pub tokens_before: u32,
    pub tokens_after: u32,
}

/// 按回合分组渲染轨迹：TurnStart 开启新回合，后续事件归入当前回合；
/// 回合前的事件（如 title_changed）忽略。工具事件按 seq 顺序归入所在回合。
pub fn render_turn_traces(events: &[StoredEvent]) -> Vec<TurnTrace> {
    let mut traces: Vec<TurnTrace> = Vec::new();

    for stored in events {
        match &stored.event {
            SessionEvent::TurnStart { turn_id } => {
                traces.push(TurnTrace {
                    turn_id: turn_id.clone(),
                    seq: stored.seq,
                    started_at: stored.created_at,
                    ended_at: None,
                    stop_reason: None,
                    user_input: None,
                    assistant_text: String::new(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    injected_context_count: 0,
                    compactions: Vec::new(),
                    token_usage: None,
                    cost: None,
                    wire_calls: Vec::new(),
                });
            }
            SessionEvent::TurnEnd { stop_reason, .. } => {
                if let Some(trace) = traces.last_mut() {
                    trace.ended_at = Some(stored.created_at);
                    trace.stop_reason = stop_reason.clone();
                }
            }
            SessionEvent::UserMessage { message, .. } => {
                if let Some(trace) = traces.last_mut() {
                    if trace.user_input.is_none() {
                        trace.user_input = Some(content_text(&message.content));
                    }
                }
            }
            SessionEvent::AssistantMessage { message, token_usage, cost } => {
                if let Some(trace) = traces.last_mut() {
                    let text = content_text(&message.content);
                    if !text.is_empty() {
                        if !trace.assistant_text.is_empty() {
                            trace.assistant_text.push('\n');
                        }
                        trace.assistant_text.push_str(&text);
                    }
                    if let Some(thinking) = extract_reasoning(message) {
                        if !trace.reasoning.is_empty() {
                            trace.reasoning.push('\n');
                        }
                        trace.reasoning.push_str(&thinking);
                    }
                    if token_usage.is_some() {
                        trace.token_usage = *token_usage;
                    }
                    if cost.is_some() {
                        trace.cost = cost.clone();
                    }
                }
            }
            SessionEvent::ContextMessage { .. } => {
                if let Some(trace) = traces.last_mut() {
                    trace.injected_context_count += 1;
                }
            }
            SessionEvent::ToolCall { call_id, tool_name, input, .. } => {
                if let Some(trace) = traces.last_mut() {
                    trace.tool_calls.push(ToolCallTrace {
                        call_id: call_id.clone(),
                        tool_name: tool_name.clone(),
                        input: input.clone(),
                        output: None,
                        is_error: false,
                        started_at: stored.created_at,
                        finished_at: None,
                    });
                }
            }
            SessionEvent::ToolResult { call_id, output, is_error, finished_at, .. } => {
                if let Some(trace) = traces.last_mut() {
                    if let Some(entry) = trace
                        .tool_calls
                        .iter_mut()
                        .find(|call| call.call_id == *call_id)
                    {
                        entry.output = Some(output.clone());
                        entry.is_error = *is_error;
                        entry.finished_at = Some(*finished_at);
                    }
                }
            }
            SessionEvent::CompactBoundary { level, tokens_before, tokens_after, .. } => {
                if let Some(trace) = traces.last_mut() {
                    trace.compactions.push(CompactionTrace {
                        level: level.clone(),
                        tokens_before: *tokens_before,
                        tokens_after: *tokens_after,
                    });
                }
            }
            SessionEvent::TitleChanged { .. } => {}
            SessionEvent::WireRequest { url, body } => {
                if let Some(trace) = traces.last_mut() {
                    trace.wire_calls.push(WireCallTrace {
                        url: url.clone(),
                        request: body.clone(),
                        response_text: None,
                        input_tokens: None,
                        output_tokens: None,
                    });
                }
            }
            SessionEvent::WireResponse { text, input_tokens, output_tokens } => {
                // 与最近一次请求配对（同一请求-流式响应顺序发生）。
                if let Some(trace) = traces.last_mut() {
                    if let Some(call) = trace.wire_calls.last_mut() {
                        call.response_text = Some(text.clone());
                        call.input_tokens = *input_tokens;
                        call.output_tokens = *output_tokens;
                    }
                }
            }
        }
    }

    traces
}
