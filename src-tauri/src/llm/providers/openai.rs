use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::time::timeout;

use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::providers::ProviderTurnResult;
use crate::llm::tools;
use crate::llm::types::{AgentMode, ContentBlock, Message, Role};
use crate::llm::utils::error_event::emit_backend_error;
use crate::llm::utils::system_prompt::load_system_prompt;

// OpenAI Provider 相关结构体定义。
// 主要负责：
// - 将 internal Message -> OpenAI JSON message
// - 触发 /v1/chat/completions?stream
// - 处理流式 SSE Delta 并 emit 到前端

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    // 目标模型名。
    model: String,
    // 发送给 OpenAI 的消息数组。
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    // 可选工具定义列表。
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    // 可选流配置。官方 OpenAI 可用 include_usage 在流末尾返回 usage。
    stream_options: Option<OpenAiStreamOptions>,
    // 是否开启流式返回。
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize, Clone)]
struct OpenAiMessage {
    // 消息角色：system/user/assistant/tool。
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Value>, // String or array of parts
    #[serde(skip_serializing_if = "Option::is_none")]
    // assistant 触发工具调用时携带的 tool_calls。
    tool_calls: Option<Vec<OpenAiReqToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    // tool 角色消息对应的调用 ID。
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct OpenAiReqToolCall {
    // 本次工具调用 ID。
    id: String,
    // 固定为 function。
    r#type: String,
    // 函数调用体。
    function: OpenAiReqFunction,
}

#[derive(Debug, Serialize, Clone)]
struct OpenAiReqFunction {
    // 工具名。
    name: String,
    // JSON 字符串化参数。
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    // 固定为 function。
    r#type: String,
    // 工具函数描述。
    function: OpenAiFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiFunction {
    // 工具名。
    name: String,
    // 工具描述。
    description: String,
    // 工具输入 schema。
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    // 本 SSE 分片中的 choices。
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    // 当前 choice 的增量 delta。
    delta: OpenAiDelta,
    // 当前 choice 的完成原因。
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    // 文本增量。
    content: Option<String>,
    // 工具调用增量。
    tool_calls: Option<Vec<OpenAiToolCall>>,
    // 兼容部分 OpenAI-compatible / reasoning 接口的推理增量字段。
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    #[allow(dead_code)]
    // tool_call 序号。
    index: usize,
    // tool_call ID 增量。
    id: Option<String>,
    // tool_call function 增量。
    function: Option<OpenAiFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunctionCall {
    // 工具函数名增量。
    name: Option<String>,
    // 工具函数参数增量。
    arguments: Option<String>,
}

fn extract_reasoning_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(extract_reasoning_text).collect::<Vec<_>>().join(""),
        Value::Object(map) => {
            for key in ["text", "content", "reasoning", "summary", "delta"] {
                if let Some(found) = map.get(key) {
                    let extracted = extract_reasoning_text(found);
                    if !extracted.is_empty() {
                        return extracted;
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

pub struct OpenAiProvider;

#[derive(Debug, Default)]
struct PendingToolCall {
    // 累积到的调用 ID。
    id: Option<String>,
    // 累积到的工具名。
    name: Option<String>,
    // 累积到的 JSON 参数字符串。
    arguments: String,
}

fn build_openai_image_part(source: &crate::llm::types::ImageSource) -> Option<Value> {
    if !source.source_type.eq_ignore_ascii_case("base64") {
        return None;
    }

    let media_type = source.media_type.trim();
    let data = source.data.trim();
    if media_type.is_empty() || data.is_empty() {
        return None;
    }

    Some(serde_json::json!({
        "type": "image_url",
        "image_url": {
            "url": format!("data:{};base64,{}", media_type, data)
        }
    }))
}

impl OpenAiProvider {
    pub async fn send_request(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, String> {
        // 读取设置并拿到当前 provider profile。
        let settings = crate::command::settings::get_settings(app.clone());
        let profile = settings.active_provider_profile();
        
        // 仅注入内置工具；MCP 采用 server 级发现，避免每轮发送全部动态工具 schema。
        let available_tools = tools::get_available_tools();

        // 加载系统提示词（含 Agent/Plan/Auto 模式逻辑）。
        let system_prompt = load_system_prompt(app, agent_mode)?;
        
        // 先注入 system 消息。
        let mut oai_messages = vec![OpenAiMessage {
            role: "system".into(),
            content: Some(Value::String(system_prompt)),
            tool_calls: None,
            tool_call_id: None,
        }];

        for m in messages {
            // 将内部角色映射到 OpenAI 角色字符串。
            let base_role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            
            match &m.content {
                crate::llm::types::Content::Text(t) => {
                    // 纯文本消息直接转换为单条 OpenAI 消息。
                    oai_messages.push(OpenAiMessage {
                        role: base_role.into(),
                        content: Some(Value::String(t.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                crate::llm::types::Content::Blocks(blocks) => {
                    // blocks 消息拆分为文本、图片、tool_calls、tool_results 四类。
                    let mut text_parts = Vec::new();
                    let mut image_parts = Vec::new();
                    let mut tool_calls = Vec::new();
                    let mut tool_results = Vec::new();
                    
                    for b in blocks {
                        match b {
                            ContentBlock::Text { text } => {
                                text_parts.push(text.clone());
                            }
                            ContentBlock::Thinking { .. } => {}
                            ContentBlock::Image { source } => {
                                if let Some(part) = build_openai_image_part(source) {
                                    image_parts.push(part);
                                }
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                // ToolUse 的 input 需要序列化为 OpenAI function.arguments 字符串。
                                let serialized_args = match serde_json::to_string(input) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        // 序列化失败时上报错误并终止本次请求。
                                        let msg = format!(
                                            "Failed to serialize tool arguments for '{}': {}",
                                            name, e
                                        );
                                        emit_backend_error(
                                            app,
                                            "llm.providers.openai",
                                            msg.clone(),
                                            Some("tool.arguments_serialize"),
                                        );
                                        return Err(msg);
                                    }
                                };
                                // 组装 assistant.tool_calls 条目。
                                tool_calls.push(OpenAiReqToolCall {
                                    id: id.clone(),
                                    r#type: "function".into(),
                                    function: OpenAiReqFunction {
                                        name: name.clone(),
                                        arguments: serialized_args,
                                    }
                                });
                            }
                            ContentBlock::ToolResult { tool_use_id, is_error: _, content } => {
                                // 将 tool_result 内所有文本块拼接为单文本。
                                let mut tr_text = Vec::new();
                                for tb in content {
                                    if let ContentBlock::Text { text } = tb {
                                        tr_text.push(text.clone());
                                    }
                                }
                                // 保留 tool_use_id 与结果文本映射。
                                tool_results.push((tool_use_id.clone(), tr_text.join("\n")));
                            }
                        }
                    }
                    
                    if base_role == "assistant" {
                        // assistant 有 tool_calls 时，content 可为空。
                        let content_val = if text_parts.is_empty() && !tool_calls.is_empty() {
                            None // Optional for tool calls in assistant
                        } else {
                            Some(Value::String(text_parts.join("\n")))
                        };
                        
                        // 仅有 tool_calls 时写入 Some(tool_calls)，否则为 None。
                        let tc = if tool_calls.is_empty() { None } else { Some(tool_calls) };
                        oai_messages.push(OpenAiMessage {
                            role: "assistant".into(),
                            content: content_val,
                            tool_calls: tc,
                            tool_call_id: None,
                        });
                    } else {
                        // User message might contain text/image/tool results.
                        if !image_parts.is_empty() {
                            let mut user_content_parts = Vec::new();
                            if !text_parts.is_empty() {
                                user_content_parts.push(serde_json::json!({
                                    "type": "text",
                                    "text": text_parts.join("\n")
                                }));
                            }
                            user_content_parts.extend(image_parts);
                            oai_messages.push(OpenAiMessage {
                                role: "user".into(),
                                content: Some(Value::Array(user_content_parts)),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        } else if !text_parts.is_empty() {
                            oai_messages.push(OpenAiMessage {
                                role: "user".into(),
                                content: Some(Value::String(text_parts.join("\n"))),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                        
                        // tool 角色消息用于回灌工具结果。
                        for (tid, tr_text) in tool_results {
                            oai_messages.push(OpenAiMessage {
                                role: "tool".into(),
                                content: Some(Value::String(tr_text)),
                                tool_calls: None,
                                tool_call_id: Some(tid),
                            });
                        }
                    }
                }
            }
        }

        // 将工具定义转换为 OpenAI function tool schema。
        let tools: Option<Vec<OpenAiTool>> = if available_tools.is_empty() {
            None
        } else {
            Some(
                available_tools
                    .into_iter()
                    .map(|t| OpenAiTool {
                        r#type: "function".into(),
                        function: OpenAiFunction {
                            name: t.name,
                            description: t.description,
                            parameters: t.input_schema,
                        },
                    })
                    .collect(),
            )
        };

        // 组装最终请求体。
        let provider_key = settings.provider.trim().to_ascii_lowercase();
        let supports_stream_usage =
            provider_key == "openai" || profile.base_url.contains("api.openai.com");
        let request = OpenAiRequest {
            model: profile.model.clone(),
            messages: oai_messages,
            tools,
            stream_options: supports_stream_usage.then_some(OpenAiStreamOptions {
                include_usage: true,
            }),
            stream: true,
        };

        // 创建 HTTP 客户端。
        let client = Client::new();
        // 规范化 base_url，确保落到 chat/completions 端点。
        let mut url = profile.base_url.trim_end_matches('/').to_string();
        if !url.ends_with("/v1/chat/completions") && !url.ends_with("/chat/completions") {
            if url.ends_with("/v1") {
                url = format!("{}/chat/completions", url);
            } else {
                url = format!("{}/v1/chat/completions", url);
            }
        }

        // 构建 POST 请求并设置 JSON content-type。
        let mut req_builder = client.post(&url).header("content-type", "application/json");

        // 存在 API key 时注入 Bearer 头。
        if !profile.api_key.is_empty() {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", profile.api_key));
        }

        // 发起请求。
        let resp = req_builder.json(&request).send().await;

        // 处理 HTTP 结果：成功走流式解析，失败返回错误。
        match resp {
            Ok(res) => {
                if !res.status().is_success() {
                    // 非 2xx 时读取响应文本并上报。
                    let status = res.status();
                    let error_text = res.text().await.unwrap_or_default();
                    eprintln!("API Error: {}", error_text);
                    let msg = format!("API Error [{}] {} => {}", status, url, error_text);
                    emit_backend_error(app, "llm.providers.openai", msg.clone(), Some("http.non_success"));
                    return Err(msg);
                }

                self.process_stream_response(app, res, conversation_id).await
            }
            Err(e) => {
                let msg = e.to_string();
                emit_backend_error(app, "llm.providers.openai", msg.clone(), Some("http.request"));
                Err(msg)
            }
        }
    }

    // 处理 OpenAI 的数据流响应。将 data chunks 按行解析并即时 emit：
    // - raw-json
    // - text (content delta)
    // - tool-use / tool-json-delta / tool-result
    // - token-usage + stop
    // 最终合成 ProviderTurnResult 供 query_engine 继续回合决策。
    async fn process_stream_response(
        &self,
        app: &AppHandle,
        response: reqwest::Response,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, String> {
        // 获取响应字节流。
        let mut stream = response.bytes_stream();
        // 累积文本输出。
        let mut generated_text = String::new();
        // 按 index 累积未完成的工具调用增量。
        let mut pending_tool_calls: BTreeMap<usize, PendingToolCall> = BTreeMap::new();
        
        // assistant 最终输出块。
        let mut output_blocks: Vec<ContentBlock> = Vec::new();
        // 工具结果块（作为下一轮 user blocks 回灌）。
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();
        // hooks 注入的附加上下文消息。
        let mut additional_context_messages: Vec<Message> = Vec::new();
        // 是否阻止后续续跑。
        let mut prevent_continuation = false;
        // hook 给出的停止原因。
        let mut hook_stop_reason: Option<String> = None;
        
        // 是否已经发过 stop 事件。
        let mut emitted_stop = false;
        // 最后一条 finish_reason。
        let mut last_finish_reason: Option<String> = None;
        // 本次请求实际输入 token（OpenAI usage.prompt_tokens）。
        let mut current_input_tokens: Option<u32> = None;
        // 本次请求实际输出 token（OpenAI usage.completion_tokens）。
        let mut current_output_tokens: Option<u32> = None;

        loop {
            // 每轮先检查是否取消。
            if crate::llm::cancellation::is_cancelled(conversation_id) {
                return Ok(ProviderTurnResult {
                    messages: Vec::new(),
                    stop_reason: Some("cancelled".into()),
                    input_tokens: current_input_tokens,
                    output_tokens: current_output_tokens,
                    prevent_continuation: false,
                });
            }

            // 200ms 轮询读取下一块，避免阻塞过久。
            let next_chunk = match timeout(Duration::from_millis(200), stream.next()).await {
                Ok(v) => v,
                Err(_) => continue,
            };

            // 流结束。
            let Some(chunk) = next_chunk else {
                break;
            };

            // 提取字节块，错误时上报并返回。
            let bytes = match chunk {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!("OpenAI stream chunk error: {}", e);
                    emit_backend_error(app, "llm.providers.openai", msg.clone(), Some("stream.chunk"));
                    return Err(msg);
                }
            };
            // 按 UTF-8 宽松解码。
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                    // 去掉行首尾空白。
                    let line = line.trim();
                    // 仅处理 SSE data 行。
                    if line.starts_with("data: ") || line.starts_with("data:") {
                        // 同时兼容 "data: " 与 "data:" 前缀。
                        let data = line.strip_prefix("data: ").unwrap_or_else(|| line.strip_prefix("data:").unwrap());
                        // 流结束标记。
                        if data == "[DONE]" {
                            break;
                        }
                        // 回传原始 JSON 便于前端调试。
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
                        // 解析 OpenAI chunk JSON。
                        if let Ok(chunk) = serde_json::from_str::<OpenAiStreamChunk>(data) {
                            if let Some(usage) = chunk.usage {
                                current_input_tokens = usage.prompt_tokens;
                                current_output_tokens = usage.completion_tokens.or_else(|| {
                                    usage
                                        .total_tokens
                                        .zip(usage.prompt_tokens)
                                        .and_then(|(total, prompt)| total.checked_sub(prompt))
                                });
                            }
                            for choice in chunk.choices {
                                let OpenAiDelta {
                                    content,
                                    tool_calls,
                                    extra,
                                } = choice.delta;
                                // 文本增量路径。
                                if let Some(content) = content {
                                    generated_text.push_str(&content);
                                    app.emit(
                                        "chat-stream",
                                        ChatMessageEvent {
                                            r#type: "text".into(),
                                            text: Some(content),
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

                                for key in ["reasoning", "reasoning_content"] {
                                    if let Some(value) = extra.get(key) {
                                        let reasoning_text = extract_reasoning_text(value);
                                        if !reasoning_text.is_empty() {
                                            app.emit(
                                                "chat-stream",
                                                ChatMessageEvent {
                                                    r#type: "reasoning".into(),
                                                    text: Some(reasoning_text),
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
                                }
                                
                                // 工具调用增量路径。
                                if let Some(tool_calls) = tool_calls {
                                    for tc in tool_calls {
                                        // 按 index 拿到并更新 pending entry。
                                        let entry = pending_tool_calls.entry(tc.index).or_default();

                                        if let Some(id) = tc.id {
                                            entry.id = Some(id);
                                        }

                                        if let Some(func) = tc.function {
                                            if let Some(name) = func.name {
                                                // 首次看到 name 时发 tool-use-start。
                                                if entry.name.is_none() {
                                                    app.emit(
                                                        "chat-stream",
                                                        ChatMessageEvent {
                                                            r#type: "tool-use-start".into(),
                                                            text: None,
                                                            tool_use_id: entry.id.clone(),
                                                            tool_use_name: Some(name.clone()),
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
                                                // 更新工具名。
                                                entry.name = Some(name);
                                            }

                                            if let Some(args) = func.arguments {
                                                // 参数增量追加到缓冲。
                                                entry.arguments.push_str(&args);
                                                app.emit(
                                                    "chat-stream",
                                                    ChatMessageEvent {
                                                        r#type: "tool-json-delta".into(),
                                                        text: None,
                                                        tool_use_id: entry.id.clone(),
                                                        tool_use_name: None,
                                                        tool_use_input: Some(args),
                                                        tool_result: None,
                                                        token_usage: None,
                                                        stop_reason: None,
                                                        turn_state: Some("tool_input_streaming".into()),
                                                        conversation_id: conversation_id.map(str::to_string),
                                                    },
                                                )
                                                .ok();
                                            }
                                        }
                                    }
                                }

                                if let Some(finish_reason) = choice.finish_reason {
                                    // 记录 finish_reason 供最终 stop_reason 使用。
                                    last_finish_reason = Some(finish_reason.clone());
                                    if finish_reason == "tool_calls" {
                                        // 把当前 pending 调用快照提取出来执行。
                                        let drained_calls: Vec<(usize, PendingToolCall)> =
                                            pending_tool_calls
                                                .iter()
                                                .map(|(k, v)| {
                                                    (
                                                        *k,
                                                        PendingToolCall {
                                                            id: v.id.clone(),
                                                            name: v.name.clone(),
                                                            arguments: v.arguments.clone(),
                                                        },
                                                    )
                                                })
                                                .collect();

                                        // 清空 pending，等待下一批增量。
                                        pending_tool_calls.clear();

                                        // 构建执行请求列表。
                                        let mut call_requests: Vec<tools::ToolCallRequest> = Vec::new();
                                        for (_, tc) in drained_calls {
                                            let (Some(id), Some(name)) = (tc.id, tc.name) else {
                                                continue;
                                            };

                                            // 反序列化参数失败时回退空对象。
                                            let input_value: Value = serde_json::from_str(&tc.arguments)
                                                .unwrap_or_else(|_| serde_json::json!({}));

                                            // 把工具调用写入 assistant 输出块。
                                            output_blocks.push(ContentBlock::ToolUse {
                                                id: id.clone(),
                                                name: name.clone(),
                                                input: input_value.clone(),
                                            });

                                            app.emit(
                                                "chat-stream",
                                                ChatMessageEvent {
                                                    r#type: "tool-executing".into(),
                                                    text: None,
                                                    tool_use_id: Some(id.clone()),
                                                    tool_use_name: Some(name.clone()),
                                                    tool_use_input: None,
                                                    tool_result: None,
                                                    token_usage: None,
                                                    stop_reason: None,
                                                    turn_state: Some("tool_executing".into()),
                                                    conversation_id: conversation_id.map(str::to_string),
                                                },
                                            )
                                            .ok();

                                            // 入队统一工具执行器。
                                            call_requests.push(tools::ToolCallRequest {
                                                id,
                                                name,
                                                input: input_value,
                                            });
                                        }

                                        // 执行工具调用（内部处理并发与hooks）。
                                        let executed_calls = tools::execute_tool_calls_with_app(
                                            app,
                                            conversation_id,
                                            call_requests,
                                        )
                                        .await;

                                        for executed in executed_calls {
                                            let serialized_input = serde_json::to_string_pretty(&executed.input)
                                                .unwrap_or_else(|_| executed.input.to_string());
                                            // 把每个工具结果实时回传前端。
                                            app.emit(
                                                "chat-stream",
                                                ChatMessageEvent {
                                                    r#type: "tool-result".into(),
                                                    text: None,
                                                    tool_use_id: Some(executed.id.clone()),
                                                    tool_use_name: Some(executed.name.clone()),
                                                    tool_use_input: Some(serialized_input),
                                                    tool_result: Some(executed.output.clone()),
                                                    token_usage: None,
                                                    stop_reason: None,
                                                    turn_state: Some("tool_completed".into()),
                                                    conversation_id: conversation_id.map(str::to_string),
                                                },
                                            )
                                            .ok();

                                            // 写入工具结果块用于下一轮回灌。
                                            tool_result_blocks.push(ContentBlock::ToolResult {
                                                tool_use_id: executed.id,
                                                is_error: executed.is_error,
                                                content: vec![ContentBlock::Text {
                                                    text: executed.output,
                                                }],
                                            });

                                            // 累积 hooks 附加消息。
                                            if !executed.additional_messages.is_empty() {
                                                additional_context_messages
                                                    .extend(executed.additional_messages);
                                            }
                                            // 累积阻断续跑标记和原因。
                                            if executed.prevent_continuation {
                                                prevent_continuation = true;
                                                if hook_stop_reason.is_none() {
                                                    hook_stop_reason = executed.stop_reason;
                                                }
                                            }
                                        }
                                    } else if finish_reason == "stop" {
                                        // OpenAI 正常 stop 时发中间 stop 事件。
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
                                                token_usage: None,
                                                stop_reason: Some(finish_reason),
                                                turn_state: Some("intermediate".into()),
                                                conversation_id: conversation_id.map(str::to_string),
                                            },
                                        )
                                        .ok();
                                    }
                                }
                            }
                        }
                    }
                }
        }
        
        // 将剩余文本写入输出块。
        if !generated_text.is_empty() {
            output_blocks.push(ContentBlock::Text {
                text: generated_text.clone(),
            });
        }

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
                    token_usage: None,
                    stop_reason: None,
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

        // 有工具结果时附加 user/tool_result 消息。
        if !tool_result_blocks.is_empty() {
            result_messages.push(Message {
                role: Role::User,
                content: crate::llm::types::Content::Blocks(tool_result_blocks),
            });
        }

        // 附加 hooks 上下文消息。
        if !additional_context_messages.is_empty() {
            result_messages.extend(additional_context_messages);
        }

        // 统一最终 stop_reason：hook 优先，其次 finish_reason。
        let final_stop_reason = if prevent_continuation {
            hook_stop_reason
                .or(last_finish_reason)
                .or_else(|| Some("hook_stopped_continuation".to_string()))
        } else {
            last_finish_reason
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
