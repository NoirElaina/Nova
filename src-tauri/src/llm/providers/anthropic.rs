use futures_util::StreamExt;
use reqwest::Client;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::time::timeout;

use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::providers::ProviderTurnResult;
use crate::llm::tools;
use crate::llm::types::{
    AnthropicRequest, ContentBlock, Message, Role, StreamContentBlock, StreamDelta, StreamEvent,
};
use crate::llm::utils::error_event::emit_backend_error;
use crate::llm::utils::system_prompt::load_system_prompt;
use crate::llm::types::AgentMode;

// Anthropic Provider 的实现结构体，用于与 Anthropic API 交互。
pub struct AnthropicProvider;

// 判断工具结果是否要求“需要用户输入”，帮助上层 query_engine 决定是否停止并等待交互。
fn is_needs_user_input_payload(raw: &str) -> bool {
    // 尝试解析工具输出 JSON 并检查 type 字段。
    serde_json::from_str::<serde_json::Value>(raw)
        // 解析失败时转 None。
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                // 仅当 type=needs_user_input 视为需要用户输入。
                .map(|s| s == "needs_user_input")
        })
        // 默认 false。
        .unwrap_or(false)
}

fn apply_tool_call_result(
    app: &AppHandle,
    conversation_id: Option<&str>,
    executed: tools::ToolCallResult,
    current_output_tokens: Option<u32>,
    stop_emitted_for_user_input: &mut bool,
    tool_result_blocks: &mut Vec<ContentBlock>,
    additional_context_messages: &mut Vec<Message>,
    prevent_continuation: &mut bool,
    hook_stop_reason: &mut Option<String>,
) {
    let serialized_input = serde_json::to_string_pretty(&executed.input)
        .unwrap_or_else(|_| executed.input.to_string());

    // 广播工具结果事件。
    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "tool-result".into(),
            text: None,
            tool_use_id: Some(executed.id.clone()),
            tool_use_name: Some(executed.name.clone()),
            tool_use_input: Some(serialized_input),
            tool_result: Some(executed.output.clone()),
            token_usage: current_output_tokens,
            stop_reason: None,
            turn_state: Some("tool_completed".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();

    // 如果工具结果要求用户输入且尚未发 stop，则发一次 awaiting_user_input。
    let needs_user_input = is_needs_user_input_payload(&executed.output);
    if needs_user_input && !*stop_emitted_for_user_input {
        *stop_emitted_for_user_input = true;
        app.emit(
            "chat-stream",
            ChatMessageEvent {
                r#type: "stop".into(),
                text: None,
                tool_use_id: None,
                tool_use_name: None,
                tool_use_input: None,
                tool_result: None,
                token_usage: current_output_tokens,
                stop_reason: Some("needs_user_input".into()),
                turn_state: Some("awaiting_user_input".into()),
                conversation_id: conversation_id.map(str::to_string),
            },
        )
        .ok();
    }

    // 把工具结果块写入回灌消息。
    tool_result_blocks.push(ContentBlock::ToolResult {
        tool_use_id: executed.id,
        is_error: executed.is_error,
        content: vec![ContentBlock::Text {
            text: executed.output,
        }],
    });

    // 合并 hooks 附加消息。
    if !executed.additional_messages.is_empty() {
        additional_context_messages.extend(executed.additional_messages);
    }

    // 合并 hooks 阻断状态与原因。
    if executed.prevent_continuation {
        *prevent_continuation = true;
        if hook_stop_reason.is_none() {
            *hook_stop_reason = executed.stop_reason;
        }
    }
}

impl AnthropicProvider {
    pub async fn send_request(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, String> {
        // 读取设置与当前 provider profile。
        let settings = crate::command::settings::get_settings(app.clone());
        let profile = settings.active_provider_profile();
        // 提取 API key。
        let api_key = profile.api_key;

        // API key 缺失时直接失败。
        if api_key.is_empty() {
            return Err("API error: No API key configured. Please set it in Settings.".to_string());
        }

        // 仅注入内置工具；MCP 采用 server 级发现，避免每轮发送全部动态工具 schema。
        let available_tools = tools::get_available_tools();

        // 构造 Anthropic 请求体。
        let request = AnthropicRequest {
            model: profile.model.clone(),
            max_tokens: 4096,
            system: Some(load_system_prompt(app, agent_mode)?),
            messages: messages.to_vec(),
            tools: available_tools,
            stream: true,
        };

        // 创建 HTTP 客户端并规范化 URL 到 /v1/messages。
        let client = Client::new();
        let mut url = profile.base_url.trim_end_matches('/').to_string();
        if !url.ends_with("/v1/messages") && !url.ends_with("/messages") {
            if url.ends_with("/v1") {
                url = format!("{}/messages", url);
            } else {
                url = format!("{}/v1/messages", url);
            }
        }

        // 发送请求并设置认证头。
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await;

        // 发起 REST 请求（stream=true），本函数本身不做流数据解析，交给 process_stream_response 处理。
        match resp {
            Ok(res) => {
                // 非 2xx 读取错误体并上报。
                if !res.status().is_success() {
                    let status = res.status();
                    let error_text = res.text().await.unwrap_or_default();
                    eprintln!("API Error: {}", error_text);
                    let msg = format!("API Error [{}] {} => {}", status, url, error_text);
                    emit_backend_error(app, "llm.providers.anthropic", msg.clone(), Some("http.non_success"));
                    return Err(msg);
                }

                self.process_stream_response(app, res, conversation_id).await
            }
            Err(e) => {
                let msg = e.to_string();
                emit_backend_error(app, "llm.providers.anthropic", msg.clone(), Some("http.request"));
                Err(msg)
            }
        }
    }

    // 处理 Anthropic 流式 SSE 响应。
    // 1) 逐行解析 data 事件；2) 立即 emit raw-json/text/tool-* 到前端；3) 组装 output blocks 用于 ProviderTurnResult。
    async fn process_stream_response(
        &self,
        app: &AppHandle,
        response: reqwest::Response,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, String> {
    // 获取响应字节流。
    let mut stream = response.bytes_stream();
    // 当前正在累积的工具调用 ID。
    let mut current_tool_id = None;
    // 当前正在累积的工具名。
    let mut current_tool_name = None;
    // 当前正在累积的工具输入 JSON 字符串。
    let mut current_tool_input = String::new();
    // 累积文本输出。
    let mut generated_text = String::new();
    // assistant 输出块集合。
    let mut output_blocks: Vec<ContentBlock> = Vec::new();
    // 工具结果块集合（作为 user 回灌）。
    let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();
    // 流内待执行工具调用批次。
    let mut pending_tool_calls: Vec<tools::ToolCallRequest> = Vec::new();
    // 是否已发过 stop 事件。
    let mut emitted_stop = false;
    // 当前输入 token 数（来自 message_start usage）。
    let mut current_input_tokens: Option<u32> = None;
    // 当前输出 token 数（来自 usage）。
    let mut current_output_tokens: Option<u32> = None;
    // 是否已经因 needs_user_input 发过 stop。
    let mut stop_emitted_for_user_input = false;
    // 最近一次 stop_reason。
    let mut last_stop_reason: Option<String> = None;
    // hooks 注入的附加上下文消息。
    let mut additional_context_messages: Vec<Message> = Vec::new();
    // 是否阻断续跑。
    let mut prevent_continuation = false;
    // hook 阻断原因。
    let mut hook_stop_reason: Option<String> = None;
    // 流内工具执行批大小（默认 2）。
    let streaming_batch_size = std::env::var("NOVA_STREAMING_TOOL_BATCH_SIZE")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2);

    loop {
        // 每轮先检查取消标记。
        if crate::llm::cancellation::is_cancelled(conversation_id) {
            return Ok(ProviderTurnResult {
                messages: Vec::new(),
                stop_reason: Some("cancelled".into()),
                input_tokens: current_input_tokens,
                output_tokens: current_output_tokens,
                prevent_continuation: false,
            });
        }

        // 200ms 轮询下一块，避免永久阻塞。
        let next_chunk = match timeout(Duration::from_millis(200), stream.next()).await {
            Ok(v) => v,
            Err(_) => continue,
        };

        // 流结束。
        let Some(chunk) = next_chunk else {
            break;
        };

        // 读取 chunk 字节，失败则返回错误。
        let bytes = match chunk {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("Anthropic stream chunk error: {}", e);
                emit_backend_error(app, "llm.providers.anthropic", msg.clone(), Some("stream.chunk"));
                return Err(msg);
            }
        };
        // 按宽松 UTF-8 解码文本。
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
                // 仅解析 data 前缀行。
                if let Some(data) = line.strip_prefix("data:") {
                    // 兼容 data: 后可选空格。
                    let data = data.trim_start();
                    // 流结束标记。
                    if data == "[DONE]" {
                        break;
                    }
                    // 回传 raw-json 给前端调试。
                    app.emit(
                        "chat-stream",
                        ChatMessageEvent {
                            r#type: "raw-json".into(),
                            text: Some(data.to_string()),
                            tool_use_id: None,
                            tool_use_name: None,
                            tool_use_input: None,
                            tool_result: None,
                            token_usage: None,
                            stop_reason: None,
                            turn_state: Some("raw_stream".into()),
                            conversation_id: conversation_id.map(str::to_string),
                        },
                    )
                    .ok();
                    // 解析为 Anthropic StreamEvent。
                    if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                        match event {
                            StreamEvent::MessageStart { message } => {
                                current_input_tokens = Some(message.usage.input_tokens);
                                current_output_tokens = Some(message.usage.output_tokens);
                            }
                            StreamEvent::ContentBlockStart { content_block, .. } => {
                                // 工具调用块开始。
                                match content_block {
                                    StreamContentBlock::ToolUse { id, name, .. } => {
                                        current_tool_id = Some(id.clone());
                                        current_tool_name = Some(name.clone());
                                        current_tool_input.clear();
                                        app.emit(
                                            "chat-stream",
                                            ChatMessageEvent {
                                                r#type: "tool-use-start".into(),
                                                text: None,
                                                tool_use_id: Some(id),
                                                tool_use_name: Some(name),
                                                tool_use_input: None,
                                                tool_result: None,
                                                token_usage: None,
                                                stop_reason: None,
                                                turn_state: Some("tool_running".into()),
                                                conversation_id: conversation_id.map(str::to_string),
                                            },
                                        )
                                        .ok();
                                    }
                                    StreamContentBlock::Thinking { thinking } => {
                                        if !thinking.is_empty() {
                                            app.emit(
                                                "chat-stream",
                                                ChatMessageEvent {
                                                    r#type: "reasoning".into(),
                                                    text: Some(thinking),
                                                    tool_use_id: None,
                                                    tool_use_name: None,
                                                    tool_use_input: None,
                                                    tool_result: None,
                                                    token_usage: None,
                                                    stop_reason: None,
                                                    turn_state: Some("streaming_reasoning".into()),
                                                    conversation_id: conversation_id.map(str::to_string),
                                                },
                                            )
                                            .ok();
                                        }
                                    }
                                    StreamContentBlock::Text { .. } => {}
                                }
                            }
                            StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                                StreamDelta::TextDelta { text } => {
                                    // 文本增量追加并回传。
                                    generated_text.push_str(&text);
                                    app.emit(
                                        "chat-stream",
                                        ChatMessageEvent {
                                            r#type: "text".into(),
                                            text: Some(text),
                                            tool_use_id: None,
                                            tool_use_name: None,
                                            tool_use_input: None,
                                            tool_result: None,
                                            token_usage: None,
                                            stop_reason: None,
                                            turn_state: Some("streaming_text".into()),
                                            conversation_id: conversation_id.map(str::to_string),
                                        },
                                    )
                                    .ok();
                                }
                                StreamDelta::ThinkingDelta { thinking } => {
                                    app.emit(
                                        "chat-stream",
                                        ChatMessageEvent {
                                            r#type: "reasoning".into(),
                                            text: Some(thinking),
                                            tool_use_id: None,
                                            tool_use_name: None,
                                            tool_use_input: None,
                                            tool_result: None,
                                            token_usage: None,
                                            stop_reason: None,
                                            turn_state: Some("streaming_reasoning".into()),
                                            conversation_id: conversation_id.map(str::to_string),
                                        },
                                    )
                                    .ok();
                                }
                                StreamDelta::SignatureDelta { .. } => {}
                                StreamDelta::InputJsonDelta { partial_json } => {
                                    // 工具输入 JSON 增量追加并回传。
                                    current_tool_input.push_str(&partial_json);
                                    app.emit(
                                        "chat-stream",
                                        ChatMessageEvent {
                                            r#type: "tool-json-delta".into(),
                                            text: None,
                                            tool_use_id: current_tool_id.clone(),
                                            tool_use_name: None,
                                            tool_use_input: Some(partial_json),
                                            tool_result: None,
                                            token_usage: None,
                                            stop_reason: None,
                                            turn_state: Some("tool_input_streaming".into()),
                                            conversation_id: conversation_id.map(str::to_string),
                                        },
                                    )
                                    .ok();
                                }
                            },
                            StreamEvent::ContentBlockStop { .. } => {
                                // 工具块结束：有工具则入队执行；否则收束文本块。
                                if let (Some(id), Some(name)) =
                                    (current_tool_id.take(), current_tool_name.take())
                                {
                                    // 解析工具输入 JSON，失败回退空对象。
                                    let input_value: serde_json::Value =
                                        serde_json::from_str(&current_tool_input)
                                            .unwrap_or_else(|_| serde_json::json!({}));

                                    // 记录 assistant 的 ToolUse 块。
                                    output_blocks.push(ContentBlock::ToolUse {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input: input_value.clone(),
                                    });

                                    // 入队等待批量执行。
                                    pending_tool_calls.push(tools::ToolCallRequest {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input: input_value,
                                    });

                                    app.emit(
                                        "chat-stream",
                                        ChatMessageEvent {
                                            r#type: "tool-executing".into(),
                                            text: None,
                                            tool_use_id: Some(id),
                                            tool_use_name: Some(name),
                                            tool_use_input: None,
                                            tool_result: None,
                                            token_usage: current_output_tokens,
                                            stop_reason: None,
                                            turn_state: Some("tool_executing".into()),
                                            conversation_id: conversation_id.map(str::to_string),
                                        },
                                    )
                                    .ok();

                                    // 达到批量阈值时执行工具。
                                    if pending_tool_calls.len() >= streaming_batch_size {
                                        let executed_calls = tools::execute_tool_calls_with_app(
                                            app,
                                            conversation_id,
                                            std::mem::take(&mut pending_tool_calls),
                                        )
                                        .await;

                                        // 把工具执行结果统一应用到状态与消息块。
                                        for executed in executed_calls {
                                            apply_tool_call_result(
                                                app,
                                                conversation_id,
                                                executed,
                                                current_output_tokens,
                                                &mut stop_emitted_for_user_input,
                                                &mut tool_result_blocks,
                                                &mut additional_context_messages,
                                                &mut prevent_continuation,
                                                &mut hook_stop_reason,
                                            );
                                        }
                                    }
                                } else if !generated_text.is_empty() {
                                    // 无工具时把累计文本落到输出块。
                                    output_blocks.push(ContentBlock::Text {
                                        text: generated_text.clone(),
                                    });
                                    generated_text.clear();
                                }
                            }
                            StreamEvent::MessageDelta { delta, usage } => {
                                // 更新 stop_reason 与 token usage。
                                if let Some(reason) = delta.stop_reason.clone() {
                                    last_stop_reason = Some(reason);
                                }
                                current_output_tokens = Some(usage.output_tokens);
                            }
                            StreamEvent::MessageStop => {
                                // message stop 前执行剩余待处理工具调用。
                                if !pending_tool_calls.is_empty() {
                                    let executed_calls = tools::execute_tool_calls_with_app(
                                        app,
                                        conversation_id,
                                        std::mem::take(&mut pending_tool_calls),
                                    )
                                    .await;
                                    // 应用剩余工具结果。
                                    for executed in executed_calls {
                                        apply_tool_call_result(
                                            app,
                                            conversation_id,
                                            executed,
                                            current_output_tokens,
                                            &mut stop_emitted_for_user_input,
                                            &mut tool_result_blocks,
                                            &mut additional_context_messages,
                                            &mut prevent_continuation,
                                            &mut hook_stop_reason,
                                        );
                                    }
                                }

                                // 标记并广播中间 stop。
                                emitted_stop = true;
                                app.emit(
                                    "chat-stream",
                                    ChatMessageEvent {
                                        r#type: "stop".into(),
                                        text: None,
                                        tool_use_id: None,
                                        tool_use_name: None,
                                        tool_use_input: None,
                                        tool_result: None,
                                        token_usage: current_output_tokens,
                                        stop_reason: last_stop_reason.clone(),
                                        turn_state: Some("intermediate".into()),
                                        conversation_id: conversation_id.map(str::to_string),
                                    },
                                )
                                .ok();
                            }
                            _ => {}
                        }
                    }
                }
            }
    }

    // 若流内没发 stop，这里补发一次。
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
                token_usage: current_output_tokens,
                stop_reason: last_stop_reason.clone(),
                turn_state: Some("intermediate".into()),
                conversation_id: conversation_id.map(str::to_string),
            },
        )
        .ok();
    }

    // 组装 assistant 消息。
    let mut result_messages = vec![Message {
        role: Role::Assistant,
        content: crate::llm::types::Content::Blocks(output_blocks),
    }];

    // 追加工具结果回灌消息。
    if !tool_result_blocks.is_empty() {
        result_messages.push(Message {
            role: Role::User,
            content: crate::llm::types::Content::Blocks(tool_result_blocks),
        });
    }

    // 追加 hooks 上下文消息。
    if !additional_context_messages.is_empty() {
        result_messages.extend(additional_context_messages);
    }

    // 统一最终 stop_reason。
    let final_stop_reason = if prevent_continuation {
        hook_stop_reason
            .or(last_stop_reason)
            .or_else(|| Some("hook_stopped_continuation".to_string()))
    } else {
        last_stop_reason
    };

    // 返回 provider 回合结果。
    Ok(ProviderTurnResult {
        messages: result_messages,
        stop_reason: final_stop_reason,
        input_tokens: current_input_tokens,
        output_tokens: current_output_tokens,
        prevent_continuation,
    })
}
}
