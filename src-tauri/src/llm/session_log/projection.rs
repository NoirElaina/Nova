//! 事件流投影：从 session_events 渲染出三种视图——
//! 1. UI 聊天历史（HistoryMessage 列表，与旧接口返回结构一致）；
//! 2. 工具执行日志（HistoryToolExecution 列表）；
//! 3. 模型上下文重建（按序拼消息，遇 CompactBoundary 重置为其 base_context）。

use crate::llm::commands::types::{HistoryMessage, HistoryToolExecution};
use crate::llm::types::{Content, ContentBlock, Role};

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
