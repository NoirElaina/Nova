//! 事件流投影：从 session_events 渲染出三种视图——
//! 1. UI 聊天历史（HistoryMessage 列表，与旧接口返回结构一致）；
//! 2. 工具执行日志（HistoryToolExecution 列表）；
//! 3. 模型上下文重建（按序拼消息，遇 CompactBoundary 重置为其 base_context）。

use crate::agent::session::types::{HistoryMessage, HistoryToolExecution};
use crate::provider::types::{Content, ContentBlock, Role};
use serde::Serialize;
use serde_json::Value;

use super::events::SessionEvent;
use super::store::StoredEvent;

/// 提取消息中的纯文本（Text 块拼接；跳过 tool/thinking 块）。
fn extract_text(message: &crate::provider::types::Message) -> String {
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
fn extract_reasoning(message: &crate::provider::types::Message) -> Option<String> {
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

/// 严格按消息中内容块（ContentBlock）的原生时间序列追加/合并至 transcriptSegments，
/// 确保流式进行中与历史重载后的时间线段落顺序 100% 像素级对齐，
/// 杜绝在同一条消息内强行把工具块挪到正文前面导致的错位与合并。
fn append_message_blocks_to_segments(
    message: &crate::provider::types::Message,
    segs: &mut Vec<Value>,
) {
    match &message.content {
        Content::Text(text) => {
            if !text.trim().is_empty() {
                if let Some(last) = segs.last_mut() {
                    if last.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(prev) = last.get("text").and_then(|t| t.as_str()) {
                            let merged = if prev.trim().is_empty() {
                                text.clone()
                            } else {
                                format!("{}\n\n{}", prev, text)
                            };
                            last["text"] = serde_json::json!(merged);
                            return;
                        }
                    }
                }
                segs.push(serde_json::json!({
                    "type": "text",
                    "text": text,
                }));
            }
        }
        Content::Blocks(blocks) => {
            for block in blocks {
                match block {
                    ContentBlock::Thinking { thinking, .. } => {
                        if !thinking.trim().is_empty() {
                            if let Some(last) = segs.last_mut() {
                                if last.get("type").and_then(|t| t.as_str()) == Some("reasoning") {
                                    if let Some(prev) = last.get("text").and_then(|t| t.as_str()) {
                                        let merged = if prev.trim().is_empty() {
                                            thinking.clone()
                                        } else {
                                            format!("{}\n\n{}", prev, thinking)
                                        };
                                        last["text"] = serde_json::json!(merged);
                                        continue;
                                    }
                                }
                            }
                            segs.push(serde_json::json!({
                                "type": "reasoning",
                                "text": thinking,
                            }));
                        }
                    }
                    ContentBlock::Text { text } => {
                        if !text.trim().is_empty() {
                            if let Some(last) = segs.last_mut() {
                                if last.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(prev) = last.get("text").and_then(|t| t.as_str()) {
                                        let merged = if prev.trim().is_empty() {
                                            text.clone()
                                        } else {
                                            format!("{}\n\n{}", prev, text)
                                        };
                                        last["text"] = serde_json::json!(merged);
                                        continue;
                                    }
                                }
                            }
                            segs.push(serde_json::json!({
                                "type": "text",
                                "text": text,
                            }));
                        }
                    }
                    ContentBlock::ToolUse { id, .. } => {
                        let call_val = serde_json::json!(id);
                        let already_exists = segs.iter().any(|seg| {
                            seg.get("type").and_then(|t| t.as_str()) == Some("tools")
                                && seg
                                    .get("toolIds")
                                    .and_then(|v| v.as_array())
                                    .is_some_and(|arr| arr.contains(&call_val))
                        });
                        if already_exists {
                            continue;
                        }

                        let mut appended = false;
                        if let Some(last) = segs.last_mut() {
                            if last.get("type").and_then(|t| t.as_str()) == Some("tools") {
                                if let Some(arr) = last.get_mut("toolIds").and_then(|v| v.as_array_mut()) {
                                    arr.push(call_val.clone());
                                    appended = true;
                                }
                            }
                        }
                        if !appended {
                            segs.push(serde_json::json!({
                                "type": "tools",
                                "toolIds": [id],
                            }));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}


/// 投影 UI 聊天历史：仅 UserMessage / AssistantMessage 进消息流，
/// id 用事件 seq（稳定唯一，前端作消息键使用）。
///
/// 同一回合的多次模型调用（thinking → tool → thinking → …）在事件流里是
/// 多条 AssistantMessage；UI 上与流式期间保持一致——合并为一条消息，
/// 由 cost.transcriptSegments 还原完整思考/工具/正文结构。
/// tool_result 回填等不可见的 UserMessage（无文本/推理/附件）既不展示也不打断合并。
pub fn render_ui_history(events: &[StoredEvent]) -> Vec<HistoryMessage> {
    let mut out: Vec<HistoryMessage> = Vec::new();
    for stored in events {
        match &stored.event {
            SessionEvent::UserMessage { message, attachments } => {
                // 旧数据兼容：早期版本把中断标记以用户消息落日志，
                // UI 不再展示这类状态消息（新数据已改以 ContextMessage 落日志）。
                let content = extract_text(message);
                if content.starts_with("[Request interrupted by") {
                    continue;
                }
                // 旧数据兼容：工具 side-channel 注入的图片消息（ReadTool/ComputerUse）
                // 曾以 UserMessage 落日志，新数据已改落 ContextMessage；
                // 这里跳过展示且不打断 assistant 合并。
                let has_inline_image = matches!(
                    &message.content,
                    Content::Blocks(blocks)
                        if blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. }))
                );
                if has_inline_image {
                    continue;
                }
                let reasoning = extract_reasoning(message);
                let has_attachments = attachments
                    .as_ref()
                    .map(|list| !list.is_empty())
                    .unwrap_or(false);
                // 不可见用户消息（tool_result 回填等）：跳过且不打断 assistant 合并。
                if content.trim().is_empty() && reasoning.is_none() && !has_attachments {
                    continue;
                }
                out.push(HistoryMessage {
                    id: Some(stored.seq),
                    role: "user".to_string(),
                    content,
                    reasoning,
                    attachments: attachments.clone(),
                    token_usage: None,
                    cost: None,
                });
            }
            SessionEvent::AssistantMessage { message, token_usage, cost } => {
                let content = extract_text(message);
                let reasoning = extract_reasoning(message);
                // 与上一条 assistant 合并：同回合工具循环的拆分输出归为一条 UI 消息。
                if let Some(last) = out.last_mut().filter(|m| m.role == "assistant") {
                    if !content.trim().is_empty() {
                        if last.content.trim().is_empty() {
                            last.content = content.clone();
                        } else {
                            last.content.push_str("\n\n");
                            last.content.push_str(&content);
                        }
                    }
                    if let Some(ref r) = reasoning {
                        match &mut last.reasoning {
                            Some(existing) if !existing.trim().is_empty() => {
                                existing.push_str("\n\n");
                                existing.push_str(r);
                            }
                            _ => last.reasoning = Some(r.clone()),
                        }
                    }
                    // token_usage 为单次调用的用量，合并时累加为回合总量。
                    if let Some(usage) = token_usage {
                        last.token_usage = Some(last.token_usage.unwrap_or(0) + *usage);
                    }
                    if let Some(new_cost) = cost {
                        let prev_segments = last.cost.as_ref()
                            .and_then(|c| c.get("transcriptSegments").cloned());
                        let mut merged_cost = new_cost.clone();
                        if let Some(prev) = prev_segments {
                            merged_cost["transcriptSegments"] = prev;
                        }
                        last.cost = Some(merged_cost);
                    }

                    // 动态维护完整的 transcriptSegments
                    let mut cost_val = last.cost.clone().unwrap_or_else(|| serde_json::json!({}));
                    let mut segs = cost_val.get("transcriptSegments")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    append_message_blocks_to_segments(message, &mut segs);
                    cost_val["transcriptSegments"] = serde_json::json!(segs);
                    last.cost = Some(cost_val);
                } else {
                    let has_tools = match &message.content {
                        crate::provider::types::Content::Blocks(blocks) => {
                            blocks.iter().any(|b| matches!(b, crate::provider::types::ContentBlock::ToolUse { .. }))
                        }
                        _ => false,
                    };

                    if !content.trim().is_empty() || reasoning.is_some() || has_tools {
                        let mut initial_segments = Vec::new();
                        append_message_blocks_to_segments(message, &mut initial_segments);

                        let mut initial_cost = cost.clone().unwrap_or_else(|| serde_json::json!({}));
                        if !initial_segments.is_empty() {
                            initial_cost["transcriptSegments"] = serde_json::json!(initial_segments);
                        }

                        out.push(HistoryMessage {
                            id: Some(stored.seq),
                            role: "assistant".to_string(),
                            content,
                            reasoning,
                            attachments: None,
                            token_usage: *token_usage,
                            cost: Some(initial_cost),
                        });
                    }
                }
            }
            SessionEvent::ToolCall { call_id, .. } => {
                let call_val = serde_json::json!(call_id);
                if let Some(last) = out.last_mut().filter(|m| m.role == "assistant") {
                    let mut cost_val = last.cost.clone().unwrap_or_else(|| serde_json::json!({}));
                    let mut segs = cost_val.get("transcriptSegments")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    let already_exists = segs.iter().any(|seg| {
                        seg.get("type").and_then(|t| t.as_str()) == Some("tools")
                            && seg.get("toolIds").and_then(|v| v.as_array()).is_some_and(|arr| arr.contains(&call_val))
                    });

                    if !already_exists {
                        let mut appended = false;
                        if let Some(last_seg) = segs.last_mut() {
                            if last_seg.get("type").and_then(|t| t.as_str()) == Some("tools") {
                                if let Some(arr) = last_seg.get_mut("toolIds").and_then(|v| v.as_array_mut()) {
                                    arr.push(call_val);
                                    appended = true;
                                }
                            }
                        }
                        if !appended {
                            segs.push(serde_json::json!({
                                "type": "tools",
                                "toolIds": [call_id],
                            }));
                        }
                        cost_val["transcriptSegments"] = serde_json::json!(segs);
                        last.cost = Some(cost_val);
                    }
                } else {
                    out.push(HistoryMessage {
                        id: Some(stored.seq),
                        role: "assistant".to_string(),
                        content: String::new(),
                        reasoning: None,
                        attachments: None,
                        token_usage: None,
                        cost: Some(serde_json::json!({
                            "transcriptSegments": [
                                {
                                    "type": "tools",
                                    "toolIds": [call_id],
                                }
                            ]
                        })),
                    });
                }
            }
            _ => {}
        }
    }

    // 为每个包含工具段的 assistant 消息自动组装持久化 toolSummary 快照，
    // 保证历史重载、脱机以及组件隔离状态下工具卡片 100% 可恢复。
    let tool_logs = render_tool_logs(events);
    for msg in out.iter_mut().filter(|m| m.role == "assistant") {
        if let Some(ref mut cost_val) = msg.cost {
            if let Some(segs) = cost_val.get("transcriptSegments").and_then(|v| v.as_array()) {
                let mut referenced_tool_ids = std::collections::HashSet::new();
                for seg in segs {
                    if seg.get("type").and_then(|t| t.as_str()) == Some("tools") {
                        if let Some(ids) = seg.get("toolIds").and_then(|v| v.as_array()) {
                            for id in ids {
                                if let Some(id_str) = id.as_str() {
                                    referenced_tool_ids.insert(id_str.to_string());
                                }
                            }
                        }
                    }
                }
                if !referenced_tool_ids.is_empty() {
                    let matching_entries: Vec<&HistoryToolExecution> = tool_logs
                        .iter()
                        .filter(|t| referenced_tool_ids.contains(&t.id))
                        .collect();
                    if !matching_entries.is_empty() {
                        let snapshot = serde_json::json!({
                            "totalCalls": matching_entries.len(),
                            "entries": matching_entries,
                        });
                        cost_val["toolSummary"] = snapshot;
                    }
                }
            }
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
                if let Some(existing) = logs.iter_mut().find(|log| log.id == *call_id) {
                    if existing.turn_id.is_none() && turn_id.is_some() {
                        existing.turn_id = turn_id.clone();
                    }
                } else {
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
            }
            SessionEvent::ToolResult { call_id, tool_name, output, is_error, started_at, finished_at } => {
                let mut matched = false;
                for entry in logs.iter_mut().filter(|log| log.id == *call_id) {
                    matched = true;
                    entry.result = output.clone();
                    entry.status = if *is_error { "error" } else { "completed" }.to_string();
                    entry.finished_at = Some(*finished_at);
                }
                if !matched {
                    logs.push(HistoryToolExecution {
                        id: call_id.clone(),
                        turn_id: None,
                        tool_name: tool_name.clone(),
                        input: String::new(),
                        result: output.clone(),
                        status: if *is_error { "error" } else { "completed" }.to_string(),
                        started_at: *started_at,
                        finished_at: Some(*finished_at),
                    });
                }
            }
            _ => {}
        }
    }

    // 终态去重兜底：以已完成/出错状态优先，严格保证每个 call_id 唯一
    let mut deduplicated: Vec<HistoryToolExecution> = Vec::with_capacity(logs.len());
    for log in logs {
        if let Some(existing) = deduplicated.iter_mut().find(|item| item.id == log.id) {
            if existing.status == "running" && log.status != "running" {
                *existing = log;
            }
        } else {
            deduplicated.push(log);
        }
    }
    deduplicated
}

/// 重建模型上下文：按序拼接消息事件，遇到 CompactBoundary 丢弃之前
/// 的全部内容并以其 base_context 为新起点。
pub fn reconstruct_model_context(events: &[StoredEvent]) -> Vec<crate::provider::types::Message> {
    let mut context: Vec<crate::provider::types::Message> = Vec::new();
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

/// 单个回合的轨迹：仅回合标识与最后一次模型收发（调试面板只展开发送/接收）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTrace {
    pub turn_id: String,
    /// 回合在事件流中的序号（排序/定位用）。
    pub seq: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub stop_reason: Option<String>,
    /// 本回合发给模型 API 的 wire 级请求/响应（按顺序配对）。
    pub wire_calls: Vec<WireCallTrace>,
}

/// 单次模型 API 请求及其响应。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCallTrace {
    /// 请求报文完整 JSON（含 system prompt/tools/消息数组）。
    pub request: Value,
    /// 流结束后的完整响应 JSON（内容块/文本/stop_reason/用量）。
    pub response: Option<Value>,
}

/// 按回合分组渲染轨迹：TurnStart 开启新回合，后续事件归入当前回合；
/// 回合前的事件（如 title_changed）忽略。只收集 wire 收发，其余事件不进轨迹。
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
                    wire_calls: Vec::new(),
                });
            }
            SessionEvent::TurnEnd { stop_reason, .. } => {
                if let Some(trace) = traces.last_mut() {
                    trace.ended_at = Some(stored.created_at);
                    trace.stop_reason = stop_reason.clone();
                }
            }
            SessionEvent::WireRequest { body, .. } => {
                if let Some(trace) = traces.last_mut() {
                    trace.wire_calls.push(WireCallTrace {
                        request: body.clone(),
                        response: None,
                    });
                }
            }
            SessionEvent::WireResponse { body, .. } => {
                // 与最近一次请求配对（同一请求-流式响应顺序发生）。
                if let Some(trace) = traces.last_mut() {
                    if let Some(call) = trace.wire_calls.last_mut() {
                        call.response = Some(body.clone());
                    }
                }
            }
            _ => {}
        }
    }

    traces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::{Content, ContentBlock, Message, Role};

    #[test]
    fn test_chronological_block_ordering_preserves_separate_tools() {
        let mut segs = Vec::new();

        // Round 1: Model thinks then calls LoadTool
        let msg1 = Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![
                ContentBlock::Thinking {
                    thinking: "thinking 1".to_string(),
                    signature: String::new(),
                },
                ContentBlock::ToolUse {
                    id: "call_load_tool".to_string(),
                    name: "LoadTool".to_string(),
                    input: serde_json::json!({}),
                },
            ]),
        };
        append_message_blocks_to_segments(&msg1, &mut segs);

        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0]["type"], "reasoning");
        assert_eq!(segs[1]["type"], "tools");
        assert_eq!(segs[1]["toolIds"], serde_json::json!(["call_load_tool"]));

        // Round 2: Model outputs text FIRST, then calls Bash 1
        let msg2 = Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![
                ContentBlock::Text {
                    text: "我将先调用工具".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "call_bash_1".to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({}),
                },
            ]),
        };
        append_message_blocks_to_segments(&msg2, &mut segs);

        // Crucial invariant: tools from round 1 and round 2 MUST NOT be merged,
        // because round 2 output text in between!
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0]["type"], "reasoning");
        assert_eq!(segs[1]["type"], "tools");
        assert_eq!(segs[1]["toolIds"], serde_json::json!(["call_load_tool"]));
        assert_eq!(segs[2]["type"], "text");
        assert_eq!(segs[2]["text"], "我将先调用工具");
        assert_eq!(segs[3]["type"], "tools");
        assert_eq!(segs[3]["toolIds"], serde_json::json!(["call_bash_1"]));
    }

    #[test]
    fn test_consecutive_parallel_tools_merged_together() {
        let mut segs = Vec::new();

        // Parallel tools in a single step (no text in between)
        let msg = Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "tool1".to_string(),
                    input: serde_json::json!({}),
                },
                ContentBlock::ToolUse {
                    id: "call_2".to_string(),
                    name: "tool2".to_string(),
                    input: serde_json::json!({}),
                },
            ]),
        };
        append_message_blocks_to_segments(&msg, &mut segs);

        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0]["type"], "tools");
        assert_eq!(segs[0]["toolIds"], serde_json::json!(["call_1", "call_2"]));
    }
}


