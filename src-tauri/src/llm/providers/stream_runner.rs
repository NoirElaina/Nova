// 所有 provider 共用的流式 SSE 运行器。
//
// 架构：
//   - `StreamParser` trait：每个 provider 实现，只负责"把一行 SSE data 解析成 Delta 列表"
//   - `Delta` enum：统一的语义事件，与具体协议无关
//   - `run_streaming`：共享的 SSE 缓冲循环 + emit + 工具执行 + ProviderTurnResult 组装
//
// 添加新 provider 只需实现 `StreamParser`，不需要写任何 SSE 循环或 emit 代码。

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use super::sse_utils::{extract_sse_data, find_sse_event_delimiter, truncate_for_log};
use crate::llm::providers::{ProviderTurnError, ProviderTurnResult};
use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::tools;
use crate::llm::types::{ContentBlock, Message, Role};
use crate::llm::utils::error_event::emit_backend_error;
use crate::llm::utils::pricing::{self, TokenUsageBreakdown, TurnCostBreakdown};

// ─────────────────────────────────────────────
// Delta — 协议无关的流语义事件
// ─────────────────────────────────────────────

/// 一个已完成的工具调用，包含解析后的输入 JSON。
#[derive(Debug)]
pub struct ReadyToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// 流式解析过程中产生的语义事件。
/// parser 的 `parse_event` / `flush` 返回此 enum 的列表，runner 负责处理。
#[derive(Debug)]
pub enum Delta {
    /// 文本内容增量。
    Text(String),
    /// 推理/思考增量（reasoning_content / thinking delta）。
    Reasoning(String),
    /// 工具调用开始（首次出现工具名）。
    ToolStart { id: Option<String>, name: String },
    /// 工具参数 JSON 增量。
    ToolArgsDelta { id: Option<String>, args: String },
    /// 一批已完整累积的工具调用，parser 决定何时 emit（触发执行）。
    ToolsReady(Vec<ReadyToolCall>),
    /// Token 用量更新。
    Usage {
        input: Option<u32>,
        output: Option<u32>,
        cache_read: Option<u32>,
        cache_creation: Option<u32>,
    },
    /// 流结束信号，附带可选 stop_reason。
    Stop { reason: Option<String> },
    /// 完整的 Anthropic thinking/reasoning 块（写入 output_blocks 供多轮上下文传递）。
    ThinkingBlock { thinking: String, signature: String },
    /// 流内发生错误，附带错误消息（runner 会上报并返回 Err）。
    Error(String),
}

// ─────────────────────────────────────────────
// StreamParser trait
// ─────────────────────────────────────────────

/// provider 协议适配层。每个 provider 实现此 trait，只需关注：
/// 1. 如何把一行 SSE `data:` 内容解析为若干 `Delta`
/// 2. 流结束后是否有需要清理的残余状态
pub trait StreamParser: Send {
    /// 解析一行 SSE data 字符串，返回零个或多个语义事件。
    /// 返回 `Err(msg)` 时 runner 会上报错误并中止。
    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String>;

    /// 流结束（bytes stream 耗尽）后调用，用于 flush 残余状态。
    /// 例如 openai 的 pending_tool_calls 如果未经 finish_reason 触发，可在此处理。
    fn flush(&mut self) -> Vec<Delta> {
        Vec::new()
    }

    /// provider 标识字符串，用于错误日志。
    fn provider_name(&self) -> &'static str;
}

// ─────────────────────────────────────────────
// AssistantOutputBuilder — 保持 assistant 内容块的流式顺序
// ─────────────────────────────────────────────

/// 维护流式 assistant 输出的内部顺序。
///
/// 文本 delta 先进入 pending buffer；任何结构块入队前都会先 flush pending 文本，
/// 保证最终 snapshot 与模型实际输出顺序一致。
#[derive(Debug, Default)]
struct AssistantOutputBuilder {
    pending_text: String,
    full_text: String,
    blocks: Vec<ContentBlock>,
}

impl AssistantOutputBuilder {
    fn append_text(&mut self, text: &str) {
        self.pending_text.push_str(text);
        self.full_text.push_str(text);
    }

    fn push_tool_use(&mut self, id: String, name: String, input: serde_json::Value) {
        self.push_block(ContentBlock::ToolUse { id, name, input });
    }

    fn push_thinking(&mut self, thinking: String, signature: String) {
        self.push_block(ContentBlock::Thinking {
            thinking,
            signature,
        });
    }

    fn full_text(&self) -> &str {
        &self.full_text
    }

    fn take_blocks(&mut self) -> Vec<ContentBlock> {
        self.flush_pending_text();
        std::mem::take(&mut self.blocks)
    }

    fn push_block(&mut self, block: ContentBlock) {
        self.flush_pending_text();
        self.blocks.push(block);
    }

    fn flush_pending_text(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending_text);
        self.blocks.push(ContentBlock::Text { text });
    }
}

// ─────────────────────────────────────────────
// run_streaming — 共享 SSE 循环
// ─────────────────────────────────────────────

/// 运行 SSE 流式响应处理循环。
///
/// 负责：
/// - SSE 字节缓冲 + 事件分帧 + UTF-8 解码
/// - 取消检查
/// - 调用 `parser.parse_event` 解析每个 data 行
/// - 将 Delta 转换为前端 `chat-stream` 事件（emit）
/// - 执行工具调用（`ToolsReady` delta 触发）
/// - 组装并返回 `ProviderTurnResult`
pub async fn run_streaming<P: StreamParser>(
    parser: &mut P,
    app: &AppHandle,
    response: reqwest::Response,
    conversation_id: Option<&str>,
    model: &str,
    cancel_token: CancellationToken,
) -> Result<ProviderTurnResult, ProviderTurnError> {
    let provider = parser.provider_name();
    let mut stream = response.bytes_stream();
    let mut sse_buffer: Vec<u8> = Vec::new();

    // assistant 输出块构建器：统一维护 Text / ToolUse / Thinking 的流式顺序。
    let mut assistant_output = AssistantOutputBuilder::default();
    // 工具结果块（下一轮作为 user 消息回灌）。
    let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();
    // hooks 注入的附加上下文消息。
    let mut additional_context_messages: Vec<Message> = Vec::new();
    // hooks 是否阻断续跑。
    let mut prevent_continuation = false;
    // hooks 给出的停止原因。
    let mut hook_stop_reason: Option<String> = None;
    // 是否已经向前端发过 stop 事件。
    let mut emitted_stop = false;
    // 流内最近一次 stop_reason。
    let mut last_stop_reason: Option<String> = None;
    // token 用量。
    let mut current_input_tokens: Option<u32> = None;
    let mut current_output_tokens: Option<u32> = None;
    let mut current_cache_read_tokens: Option<u32> = None;
    let mut current_cache_creation_tokens: Option<u32> = None;

    // ── 主循环 ──────────────────────────────────────────────────────
    loop {
        // 取消信号即时响应：token.cancelled() 在 request_cancel 调用 token.cancel() 后立刻返回，
        // 无需轮询等待。
        let next_chunk = tokio::select! {
            chunk = stream.next() => chunk,
            _ = cancel_token.cancelled() => {
                // 把已流式输出的部分内容封装成 partial assistant 消息返回，
                // 确保 turn_snapshot 与 UI 历史（conversation_messages）保持一致。
                let partial_messages = build_partial_cancelled_messages(
                    &mut assistant_output,
                    &mut tool_result_blocks,
                    &mut additional_context_messages,
                );
                return Ok(ProviderTurnResult {
                    messages: partial_messages,
                    stop_reason: Some("cancelled".into()),
                    input_tokens: current_input_tokens,
                    output_tokens: current_output_tokens,
                    cache_read_tokens: current_cache_read_tokens,
                    cache_creation_tokens: current_cache_creation_tokens,
                    cost: current_turn_cost(
                        provider,
                        model,
                        current_input_tokens,
                        current_output_tokens,
                        current_cache_read_tokens,
                        current_cache_creation_tokens,
                    ),
                    prevent_continuation: false,
                });
            }
        };

        // 流结束。
        let Some(chunk) = next_chunk else {
            break;
        };

        // 读取字节 chunk，失败时上报并返回错误。
        let bytes = match chunk {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{} stream chunk error: {}", provider, e);
                emit_backend_error(
                    app,
                    &format!("llm.providers.{}", provider),
                    msg.clone(),
                    Some("stream.chunk"),
                );
                return Err(ProviderTurnError::with_partial(
                    msg,
                    build_partial_cancelled_messages(
                        &mut assistant_output,
                        &mut tool_result_blocks,
                        &mut additional_context_messages,
                    ),
                ));
            }
        };
        sse_buffer.extend_from_slice(&bytes);

        // 在缓冲区内消费所有完整 SSE 事件。
        while let Some((event_end, delimiter_len)) = find_sse_event_delimiter(&sse_buffer) {
            let event_bytes = sse_buffer[..event_end].to_vec();
            sse_buffer.drain(..event_end + delimiter_len);

            // UTF-8 解码失败时上报并中止。
            let event_raw = match String::from_utf8(event_bytes) {
                Ok(s) => s,
                Err(e) => {
                    let preview = String::from_utf8_lossy(e.as_bytes()).into_owned();
                    let msg = format!(
                        "{} stream returned non-UTF-8 SSE event. Preview: {}",
                        provider,
                        truncate_for_log(&preview, 800)
                    );
                    emit_backend_error(
                        app,
                        &format!("llm.providers.{}", provider),
                        msg.clone(),
                        Some("stream.utf8"),
                    );
                    return Err(ProviderTurnError::with_partial(
                        msg,
                        build_partial_cancelled_messages(
                            &mut assistant_output,
                            &mut tool_result_blocks,
                            &mut additional_context_messages,
                        ),
                    ));
                }
            };

            let data = extract_sse_data(&event_raw);
            if data.is_empty() || data == "[DONE]" {
                continue;
            }

            // 调用 provider parser 解析 data 行 → Delta 列表。
            let deltas = match parser.parse_event(&data) {
                Ok(d) => d,
                Err(e) => {
                    emit_backend_error(
                        app,
                        &format!("llm.providers.{}", provider),
                        e.clone(),
                        Some("stream.parse"),
                    );
                    return Err(ProviderTurnError::with_partial(
                        e,
                        build_partial_cancelled_messages(
                            &mut assistant_output,
                            &mut tool_result_blocks,
                            &mut additional_context_messages,
                        ),
                    ));
                }
            };

            // 处理每个 Delta。
            for delta in deltas {
                if let Err(e) = process_delta(
                    delta,
                    app,
                    conversation_id,
                    provider,
                    &mut assistant_output,
                    &mut tool_result_blocks,
                    &mut additional_context_messages,
                    &mut prevent_continuation,
                    &mut hook_stop_reason,
                    &mut emitted_stop,
                    &mut last_stop_reason,
                    &mut current_input_tokens,
                    &mut current_output_tokens,
                    &mut current_cache_read_tokens,
                    &mut current_cache_creation_tokens,
                )
                .await
                {
                    return Err(ProviderTurnError::with_partial(
                        e,
                        build_partial_cancelled_messages(
                            &mut assistant_output,
                            &mut tool_result_blocks,
                            &mut additional_context_messages,
                        ),
                    ));
                }
            }
        }
    }

    // 流结束后 flush parser 残余状态。
    let flush_deltas = parser.flush();
    for delta in flush_deltas {
        if let Err(e) = process_delta(
            delta,
            app,
            conversation_id,
            provider,
            &mut assistant_output,
            &mut tool_result_blocks,
            &mut additional_context_messages,
            &mut prevent_continuation,
            &mut hook_stop_reason,
            &mut emitted_stop,
            &mut last_stop_reason,
            &mut current_input_tokens,
            &mut current_output_tokens,
            &mut current_cache_read_tokens,
            &mut current_cache_creation_tokens,
        )
        .await
        {
            return Err(ProviderTurnError::with_partial(
                e,
                build_partial_cancelled_messages(
                    &mut assistant_output,
                    &mut tool_result_blocks,
                    &mut additional_context_messages,
                ),
            ));
        }
    }

    // 检查缓冲区是否有未消费的残余字节（不应出现）。
    if !sse_buffer.iter().all(u8::is_ascii_whitespace) {
        let preview = String::from_utf8_lossy(&sse_buffer).into_owned();
        let msg = format!(
            "{} stream ended with incomplete SSE event buffered. pending_bytes={}, preview={}",
            provider,
            sse_buffer.len(),
            truncate_for_log(&preview, 800)
        );
        emit_backend_error(
            app,
            &format!("llm.providers.{}", provider),
            msg.clone(),
            Some("stream.incomplete_event"),
        );
        return Err(ProviderTurnError::with_partial(
            msg,
            build_partial_cancelled_messages(
                &mut assistant_output,
                &mut tool_result_blocks,
                &mut additional_context_messages,
            ),
        ));
    }

    // 将剩余 pending 文本按顺序写入输出块。
    let output_blocks = assistant_output.take_blocks();

    // 若流内未发 stop，这里补发一次。
    if !emitted_stop {
        app.emit(
            "chat-stream",
            ChatMessageEvent {
                r#type: "stop".into(),
                text: None,
                tool_use_id: None,
                tool_use_name: None,
                tool_use_input: None,
                tool_result: None,
                tool_is_error: None,
                token_usage: current_output_tokens,
                stop_reason: last_stop_reason.clone(),
                turn_state: Some("intermediate".into()),
                conversation_id: conversation_id.map(str::to_string),
            },
        )
        .ok();
    }

    let output_blocks_empty = output_blocks.is_empty();
    let tool_result_blocks_empty = tool_result_blocks.is_empty();

    // 组装 assistant 消息。
    let mut result_messages = vec![Message {
        role: Role::Assistant,
        content: crate::llm::types::Content::Blocks(output_blocks),
    }];

    // 有工具结果时追加 user/tool_result 消息。
    if !tool_result_blocks.is_empty() {
        result_messages.push(Message {
            role: Role::User,
            content: crate::llm::types::Content::Blocks(tool_result_blocks),
        });
    }

    // 追加 hooks 附加上下文消息。
    if !additional_context_messages.is_empty() {
        result_messages.extend(additional_context_messages);
    }

    // 统一 stop_reason：hook 优先，其次流内 finish_reason，兜底 "hook_stopped_continuation"。
    let final_stop_reason = if prevent_continuation {
        hook_stop_reason
            .or(last_stop_reason)
            .or_else(|| Some("hook_stopped_continuation".to_string()))
    } else {
        last_stop_reason
    };

    // @@日志记录 wire_response — 记录 AI 完整回复文本及 token 用量（流结束后写一次）。
    if !assistant_output.full_text().is_empty() {
        crate::llm::utils::turn_log::log_wire_response(
            app,
            conversation_id,
            assistant_output.full_text(),
            current_input_tokens,
            current_output_tokens,
        );
    }

    if output_blocks_empty && tool_result_blocks_empty {
        let msg = format!(
            "{} provider returned empty assistant message. stop_reason={:?}, input_tokens={:?}, output_tokens={:?}",
            provider, final_stop_reason, current_input_tokens, current_output_tokens
        );
        emit_backend_error(
            app,
            &format!("llm.providers.{}", provider),
            msg.clone(),
            Some("stream.empty_assistant"),
        );
        return Err(ProviderTurnError::new(msg));
    }

    Ok(ProviderTurnResult {
        messages: result_messages,
        stop_reason: final_stop_reason,
        input_tokens: current_input_tokens,
        output_tokens: current_output_tokens,
        cache_read_tokens: current_cache_read_tokens,
        cache_creation_tokens: current_cache_creation_tokens,
        cost: current_turn_cost(
            provider,
            model,
            current_input_tokens,
            current_output_tokens,
            current_cache_read_tokens,
            current_cache_creation_tokens,
        ),
        prevent_continuation,
    })
}

// ─────────────────────────────────────────────
// build_partial_cancelled_messages — 取消时组装已输出的部分 assistant 消息
// ─────────────────────────────────────────────

/// 中断（取消或错误）时将已积累的流式输出打包成消息列表返回，
/// 使 turn_snapshot 与前端 conversation_messages 保持一致。
/// - 若 output_blocks 含 ToolUse，必须同时携带 tool_result_blocks，
///   否则 snapshot 会处于"有 ToolUse 无 ToolResult"的非法状态。
/// - 若工具产生了 side-channel 上下文（例如截图 image message），也必须一并携带，
///   否则 ToolResult 会声称 attached_to_context=true，但真正的上下文消息已丢失。
/// - 若尚无任何内容，返回空 Vec（query.rs 侧会补 [Request interrupted by user]）。
fn build_partial_cancelled_messages(
    assistant_output: &mut AssistantOutputBuilder,
    tool_result_blocks: &mut Vec<ContentBlock>,
    additional_context_messages: &mut Vec<Message>,
) -> Vec<Message> {
    let output_blocks = assistant_output.take_blocks();
    if output_blocks.is_empty() {
        return Vec::new();
    }
    let mut messages = vec![Message {
        role: Role::Assistant,
        content: crate::llm::types::Content::Blocks(output_blocks),
    }];
    // 有 tool_result 时一并打包，保证 ToolUse/ToolResult 成对出现。
    if !tool_result_blocks.is_empty() {
        messages.push(Message {
            role: Role::User,
            content: crate::llm::types::Content::Blocks(std::mem::take(tool_result_blocks)),
        });
    }
    // 与正常完成路径保持一致：工具 side-channel 消息也是模型上下文的一部分。
    if !additional_context_messages.is_empty() {
        messages.extend(std::mem::take(additional_context_messages));
    }
    messages
}

// ─────────────────────────────────────────────
// process_delta — 处理单个 Delta
// ─────────────────────────────────────────────

/// 处理一个 `Delta`：更新状态、emit 前端事件、执行工具调用。
#[allow(clippy::too_many_arguments)]
async fn process_delta(
    delta: Delta,
    app: &AppHandle,
    conversation_id: Option<&str>,
    provider: &str,
    assistant_output: &mut AssistantOutputBuilder,
    tool_result_blocks: &mut Vec<ContentBlock>,
    additional_context_messages: &mut Vec<Message>,
    prevent_continuation: &mut bool,
    hook_stop_reason: &mut Option<String>,
    emitted_stop: &mut bool,
    last_stop_reason: &mut Option<String>,
    current_input_tokens: &mut Option<u32>,
    current_output_tokens: &mut Option<u32>,
    current_cache_read_tokens: &mut Option<u32>,
    current_cache_creation_tokens: &mut Option<u32>,
) -> Result<(), String> {
    match delta {
        Delta::Text(text) => {
            assistant_output.append_text(&text);
            crate::llm::services::live_turns::append_text(conversation_id, &text);
            app.emit(
                "chat-stream",
                ChatMessageEvent {
                    r#type: "text".into(),
                    text: Some(text),
                    tool_use_id: None,
                    tool_use_name: None,
                    tool_use_input: None,
                    tool_result: None,
                    tool_is_error: None,
                    token_usage: None,
                    stop_reason: None,
                    turn_state: Some("streaming_text".into()),
                    conversation_id: conversation_id.map(str::to_string),
                },
            )
            .ok();
        }

        Delta::Reasoning(text) => {
            crate::llm::services::live_turns::append_reasoning(conversation_id, &text);
            app.emit(
                "chat-stream",
                ChatMessageEvent {
                    r#type: "reasoning".into(),
                    text: Some(text),
                    tool_use_id: None,
                    tool_use_name: None,
                    tool_use_input: None,
                    tool_result: None,
                    tool_is_error: None,
                    token_usage: None,
                    stop_reason: None,
                    turn_state: Some("streaming_reasoning".into()),
                    conversation_id: conversation_id.map(str::to_string),
                },
            )
            .ok();
        }

        Delta::ToolStart { id, name } => {
            app.emit(
                "chat-stream",
                ChatMessageEvent {
                    r#type: "tool-use-start".into(),
                    text: None,
                    tool_use_id: id,
                    tool_use_name: Some(name),
                    tool_use_input: None,
                    tool_result: None,
                    tool_is_error: None,
                    token_usage: None,
                    stop_reason: None,
                    turn_state: Some("tool_running".into()),
                    conversation_id: conversation_id.map(str::to_string),
                },
            )
            .ok();
        }

        Delta::ToolArgsDelta { id, args } => {
            app.emit(
                "chat-stream",
                ChatMessageEvent {
                    r#type: "tool-json-delta".into(),
                    text: None,
                    tool_use_id: id,
                    tool_use_name: None,
                    tool_use_input: Some(args),
                    tool_result: None,
                    tool_is_error: None,
                    token_usage: None,
                    stop_reason: None,
                    turn_state: Some("tool_input_streaming".into()),
                    conversation_id: conversation_id.map(str::to_string),
                },
            )
            .ok();
        }

        Delta::ToolsReady(ready_calls) => {
            // 先将所有工具写入 assistant blocks，再批量执行。
            let mut call_requests: Vec<tools::ToolCallRequest> = Vec::new();
            for call in ready_calls {
                assistant_output.push_tool_use(
                    call.id.clone(),
                    call.name.clone(),
                    call.input.clone(),
                );
                app.emit(
                    "chat-stream",
                    ChatMessageEvent {
                        r#type: "tool-executing".into(),
                        text: None,
                        tool_use_id: Some(call.id.clone()),
                        tool_use_name: Some(call.name.clone()),
                        tool_use_input: None,
                        tool_result: None,
                        tool_is_error: None,
                        token_usage: *current_output_tokens,
                        stop_reason: None,
                        turn_state: Some("tool_executing".into()),
                        conversation_id: conversation_id.map(str::to_string),
                    },
                )
                .ok();
                call_requests.push(tools::ToolCallRequest {
                    id: call.id,
                    name: call.name,
                    input: call.input,
                });
            }

            let executed_calls =
                tools::execute_tool_calls_with_app(app, conversation_id, call_requests).await;

            for executed in executed_calls {
                let serialized_input = serde_json::to_string_pretty(&executed.input)
                    .unwrap_or_else(|_| executed.input.to_string());

                app.emit(
                    "chat-stream",
                    ChatMessageEvent {
                        r#type: "tool-result".into(),
                        text: None,
                        tool_use_id: Some(executed.id.clone()),
                        tool_use_name: Some(executed.name.clone()),
                        tool_use_input: Some(serialized_input),
                        tool_result: Some(executed.output.clone()),
                        tool_is_error: Some(executed.is_error),
                        token_usage: *current_output_tokens,
                        stop_reason: None,
                        turn_state: Some("tool_completed".into()),
                        conversation_id: conversation_id.map(str::to_string),
                    },
                )
                .ok();

                // Anthropic 特有：工具结果为 needs_user_input 时补发 stop。
                if is_needs_user_input_payload(&executed.output) {
                    app.emit(
                        "chat-stream",
                        ChatMessageEvent {
                            r#type: "stop".into(),
                            text: None,
                            tool_use_id: None,
                            tool_use_name: None,
                            tool_use_input: None,
                            tool_result: None,
                            tool_is_error: None,
                            token_usage: *current_output_tokens,
                            stop_reason: Some("needs_user_input".into()),
                            turn_state: Some("awaiting_user_input".into()),
                            conversation_id: conversation_id.map(str::to_string),
                        },
                    )
                    .ok();
                }

                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: executed.id,
                    is_error: executed.is_error,
                    content: vec![ContentBlock::Text {
                        text: executed.output,
                    }],
                });

                if !executed.additional_messages.is_empty() {
                    additional_context_messages.extend(executed.additional_messages);
                }
                if executed.prevent_continuation {
                    *prevent_continuation = true;
                    if hook_stop_reason.is_none() {
                        *hook_stop_reason = executed.stop_reason;
                    }
                }
            }
        }

        Delta::Usage {
            input,
            output,
            cache_read,
            cache_creation,
        } => {
            if let Some(v) = input {
                *current_input_tokens = Some(v);
            }
            if let Some(v) = output {
                *current_output_tokens = Some(v);
            }
            if let Some(v) = cache_read {
                *current_cache_read_tokens = Some(v);
            }
            if let Some(v) = cache_creation {
                *current_cache_creation_tokens = Some(v);
            }
        }

        Delta::Stop { reason } => {
            if reason.is_some() {
                *last_stop_reason = reason.clone();
            }
            *emitted_stop = true;
            app.emit(
                "chat-stream",
                ChatMessageEvent {
                    r#type: "stop".into(),
                    text: None,
                    tool_use_id: None,
                    tool_use_name: None,
                    tool_use_input: None,
                    tool_result: None,
                    tool_is_error: None,
                    token_usage: *current_output_tokens,
                    stop_reason: reason,
                    turn_state: Some("intermediate".into()),
                    conversation_id: conversation_id.map(str::to_string),
                },
            )
            .ok();
        }

        Delta::ThinkingBlock {
            thinking,
            signature,
        } => {
            assistant_output.push_thinking(thinking, signature);
        }

        Delta::Error(msg) => {
            emit_backend_error(
                app,
                &format!("llm.providers.{}", provider),
                msg.clone(),
                Some("stream.provider_error"),
            );
            return Err(msg);
        }
    }
    Ok(())
}

fn current_turn_cost(
    provider: &str,
    model: &str,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    cache_read_tokens: Option<u32>,
    cache_creation_tokens: Option<u32>,
) -> Option<TurnCostBreakdown> {
    if model.trim().is_empty() {
        return None;
    }
    let usage = TokenUsageBreakdown {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    };
    pricing::calculate_for_model(
        &model,
        &usage,
        pricing::cache_billing_for_provider(provider),
    )
}

/// 工具结果是否表示需要用户输入（type == "needs_user_input"）。
fn is_needs_user_input_payload(raw: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|s| s == "needs_user_input")
        })
        .unwrap_or(false)
}
