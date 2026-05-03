use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
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

const STREAM_DIAGNOSTIC_EVENT_LIMIT: usize = 20;
const STREAM_DIAGNOSTIC_PREVIEW_CHARS: usize = 240;

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

fn truncate_for_log(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", preview)
    } else {
        preview
    }
}

#[derive(Debug, Clone)]
struct StreamDiagnosticEvent {
    seq: u64,
    parse_ok: bool,
    parse_error: Option<String>,
    data_preview: String,
    choices_len: Option<usize>,
    finish_reasons: Vec<String>,
    tool_delta_count: usize,
    tool_delta_summaries: Vec<String>,
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_sse_event_delimiter(input: &[u8]) -> Option<(usize, usize)> {
    let lf = find_bytes(input, b"\n\n").map(|idx| (idx, 2));
    let crlf = find_bytes(input, b"\r\n\r\n").map(|idx| (idx, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn extract_sse_data(event_raw: &str) -> String {
    event_raw
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix("data:")
                .map(|data| data.trim_start().to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_tool_delta(tool_call: &OpenAiToolCall) -> String {
    let name = tool_call
        .function
        .as_ref()
        .and_then(|function| function.name.as_deref())
        .unwrap_or("-");
    let arguments_len = tool_call
        .function
        .as_ref()
        .and_then(|function| function.arguments.as_ref())
        .map(|arguments| arguments.len())
        .unwrap_or(0);
    format!(
        "index={} has_id={} name={} arguments_len={}",
        tool_call.index,
        tool_call.id.is_some(),
        name,
        arguments_len
    )
}

fn build_stream_diagnostic_event(
    seq: u64,
    data: &str,
    parsed: Result<&OpenAiStreamChunk, String>,
) -> StreamDiagnosticEvent {
    match parsed {
        Ok(chunk) => {
            let finish_reasons = chunk
                .choices
                .iter()
                .filter_map(|choice| choice.finish_reason.clone())
                .collect::<Vec<_>>();
            let tool_delta_summaries = chunk
                .choices
                .iter()
                .filter_map(|choice| choice.delta.tool_calls.as_ref())
                .flat_map(|tool_calls| tool_calls.iter().map(summarize_tool_delta))
                .collect::<Vec<_>>();
            StreamDiagnosticEvent {
                seq,
                parse_ok: true,
                parse_error: None,
                data_preview: truncate_for_log(data, STREAM_DIAGNOSTIC_PREVIEW_CHARS),
                choices_len: Some(chunk.choices.len()),
                finish_reasons,
                tool_delta_count: tool_delta_summaries.len(),
                tool_delta_summaries,
            }
        }
        Err(error) => StreamDiagnosticEvent {
            seq,
            parse_ok: false,
            parse_error: Some(error),
            data_preview: truncate_for_log(data, STREAM_DIAGNOSTIC_PREVIEW_CHARS),
            choices_len: None,
            finish_reasons: Vec::new(),
            tool_delta_count: 0,
            tool_delta_summaries: Vec::new(),
        },
    }
}

fn push_stream_diagnostic_event(
    recent_events: &mut VecDeque<StreamDiagnosticEvent>,
    event: StreamDiagnosticEvent,
) {
    if recent_events.len() >= STREAM_DIAGNOSTIC_EVENT_LIMIT {
        recent_events.pop_front();
    }
    recent_events.push_back(event);
}

fn format_stream_diagnostics(
    recent_events: &VecDeque<StreamDiagnosticEvent>,
    pending_buffer_bytes: usize,
) -> String {
    if recent_events.is_empty() {
        return format!(
            "recent_sse_events=[] pending_buffer_bytes={}",
            pending_buffer_bytes
        );
    }

    let events = recent_events
        .iter()
        .map(|event| {
            format!(
                "#{} ok={} choices={:?} finish={:?} tool_delta_count={} tool_deltas={:?} parse_error={:?} data={}",
                event.seq,
                event.parse_ok,
                event.choices_len,
                event.finish_reasons,
                event.tool_delta_count,
                event.tool_delta_summaries,
                event.parse_error,
                event.data_preview
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");

    format!(
        "recent_sse_events=[{}] pending_buffer_bytes={}",
        events, pending_buffer_bytes
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sse_event_delimiters() {
        assert_eq!(find_sse_event_delimiter(b"data: {}\n\nrest"), Some((8, 2)));
        assert_eq!(
            find_sse_event_delimiter(b"data: {}\r\n\r\nrest"),
            Some((8, 4))
        );
    }

    #[test]
    fn waits_for_complete_sse_event_before_parsing() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(b"data: {\"choices\":[");
        assert_eq!(find_sse_event_delimiter(&buffer), None);

        buffer.extend_from_slice(b"]}\n\n");
        let (event_idx, delimiter_len) = find_sse_event_delimiter(&buffer).unwrap();
        assert_eq!(delimiter_len, 2);

        let event_raw = String::from_utf8(buffer[..event_idx].to_vec()).unwrap();
        assert_eq!(extract_sse_data(&event_raw), "{\"choices\":[]}");
    }

    #[test]
    fn extracts_multiline_sse_data() {
        let raw = "event: message\ndata: {\"a\":\ndata: 1}\n";
        assert_eq!(extract_sse_data(raw), "{\"a\":\n1}");
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
                        // 必须先回灌 tool 角色消息（OpenAI 要求 tool_result 紧跟在 assistant 的 tool_calls 之后）。
                        for (tid, tr_text) in tool_results {
                            oai_messages.push(OpenAiMessage {
                                role: "tool".into(),
                                content: Some(Value::String(tr_text)),
                                tool_calls: None,
                                tool_call_id: Some(tid),
                            });
                        }

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

    // 处理 OpenAI 的数据流响应。将网络 chunk 缓冲成完整 SSE event 后再解析并即时 emit：
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
        // SSE event 和 UTF-8 字符都可能跨网络 chunk，必须按字节缓冲到完整 event 再解码。
        let mut sse_buffer: Vec<u8> = Vec::new();
        let mut sse_next_seq = 0u64;
        let mut recent_sse_events: VecDeque<StreamDiagnosticEvent> =
            VecDeque::with_capacity(STREAM_DIAGNOSTIC_EVENT_LIMIT);

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
            sse_buffer.extend_from_slice(&bytes);
            while let Some((event_idx, delimiter_len)) = find_sse_event_delimiter(&sse_buffer) {
                let event_bytes = sse_buffer[..event_idx].to_vec();
                sse_buffer.drain(..event_idx + delimiter_len);

                let event_raw = match String::from_utf8(event_bytes) {
                    Ok(event_raw) => event_raw,
                    Err(e) => {
                        sse_next_seq += 1;
                        let invalid_bytes = e.into_bytes();
                        let preview = String::from_utf8_lossy(&invalid_bytes).into_owned();
                        let msg = format!(
                            "OpenAI stream returned a non-UTF-8 SSE event. Event preview: {}. {}",
                            truncate_for_log(&preview, 800),
                            format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
                        );
                        push_stream_diagnostic_event(
                            &mut recent_sse_events,
                            StreamDiagnosticEvent {
                                seq: sse_next_seq,
                                parse_ok: false,
                                parse_error: Some("non-UTF-8 SSE event".to_string()),
                                data_preview: truncate_for_log(&preview, STREAM_DIAGNOSTIC_PREVIEW_CHARS),
                                choices_len: None,
                                finish_reasons: Vec::new(),
                                tool_delta_count: 0,
                                tool_delta_summaries: Vec::new(),
                            },
                        );
                        emit_backend_error(
                            app,
                            "llm.providers.openai",
                            msg.clone(),
                            Some("stream.utf8"),
                        );
                        return Err(msg);
                    }
                };

                let data = extract_sse_data(&event_raw);
                if data.is_empty() {
                    continue;
                }

                sse_next_seq += 1;
                if data == "[DONE]" {
                    push_stream_diagnostic_event(
                        &mut recent_sse_events,
                        StreamDiagnosticEvent {
                            seq: sse_next_seq,
                            parse_ok: true,
                            parse_error: None,
                            data_preview: "[DONE]".to_string(),
                            choices_len: Some(0),
                            finish_reasons: vec!["[DONE]".to_string()],
                            tool_delta_count: 0,
                            tool_delta_summaries: Vec::new(),
                        },
                    );
                    continue;
                }

                app.emit(
                    "chat-stream",
                    ChatMessageEvent {
                        r#type: "raw-json".into(),
                        text: Some(data.clone()),
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

                let chunk = match serde_json::from_str::<OpenAiStreamChunk>(&data) {
                    Ok(chunk) => {
                        push_stream_diagnostic_event(
                            &mut recent_sse_events,
                            build_stream_diagnostic_event(sse_next_seq, &data, Ok(&chunk)),
                        );
                        chunk
                    }
                    Err(e) => {
                        let parse_error = e.to_string();
                        let msg = format!(
                            "Failed to parse OpenAI SSE event JSON: {}. Data preview: {}. {}",
                            parse_error,
                            truncate_for_log(&data, 800),
                            format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
                        );
                        push_stream_diagnostic_event(
                            &mut recent_sse_events,
                            build_stream_diagnostic_event(
                                sse_next_seq,
                                &data,
                                Err(parse_error.clone()),
                            ),
                        );
                        emit_backend_error(
                            app,
                            "llm.providers.openai",
                            msg.clone(),
                            Some("stream.parse"),
                        );
                        return Err(msg);
                    }
                };

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

                    if let Some(tool_calls) = tool_calls {
                        for tc in tool_calls {
                            let entry = pending_tool_calls.entry(tc.index).or_default();

                            if let Some(id) = tc.id {
                                entry.id = Some(id);
                            }

                            if let Some(func) = tc.function {
                                if let Some(name) = func.name {
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
                                    entry.name = Some(name);
                                }

                                if let Some(args) = func.arguments {
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
                        last_finish_reason = Some(finish_reason.clone());
                        if finish_reason == "tool_calls" {
                            let drained_calls: Vec<(usize, PendingToolCall)> = pending_tool_calls
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

                            pending_tool_calls.clear();
                            if drained_calls.is_empty() {
                                let msg = format!(
                                    "OpenAI stream reported finish_reason=tool_calls, but no pending tool call deltas were captured. {}",
                                    format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
                                );
                                emit_backend_error(
                                    app,
                                    "llm.providers.openai",
                                    msg.clone(),
                                    Some("stream.tool_calls.empty_pending"),
                                );
                                return Err(msg);
                            }

                            let mut call_requests: Vec<tools::ToolCallRequest> = Vec::new();
                            for (index, tc) in drained_calls {
                                let raw_arguments = tc.arguments;
                                let (id, name) = match (tc.id, tc.name) {
                                    (Some(id), Some(name)) => (id, name),
                                    (id, name) => {
                                        let msg = format!(
                                            "OpenAI tool call delta at index {} was incomplete when finish_reason=tool_calls arrived: has_id={}, has_name={}, arguments_preview={}. {}",
                                            index,
                                            id.is_some(),
                                            name.is_some(),
                                            truncate_for_log(&raw_arguments, 800),
                                            format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
                                        );
                                        emit_backend_error(
                                            app,
                                            "llm.providers.openai",
                                            msg.clone(),
                                            Some("stream.tool_calls.incomplete"),
                                        );
                                        return Err(msg);
                                    }
                                };

                                let input_value: Value = match serde_json::from_str(&raw_arguments) {
                                    Ok(value) => value,
                                    Err(e) => {
                                        let msg = format!(
                                            "Failed to parse OpenAI tool call arguments for '{}': {}. Raw arguments preview: {}. {}",
                                            name,
                                            e,
                                            truncate_for_log(&raw_arguments, 800),
                                            format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
                                        );
                                        emit_backend_error(
                                            app,
                                            "llm.providers.openai",
                                            msg.clone(),
                                            Some("stream.tool_arguments.parse"),
                                        );
                                        return Err(msg);
                                    }
                                };

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

                                call_requests.push(tools::ToolCallRequest {
                                    id,
                                    name,
                                    input: input_value,
                                });
                            }

                            if call_requests.is_empty() {
                                let msg = format!(
                                    "OpenAI stream reported finish_reason=tool_calls, but no complete tool calls were queued for execution. {}",
                                    format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
                                );
                                emit_backend_error(
                                    app,
                                    "llm.providers.openai",
                                    msg.clone(),
                                    Some("stream.tool_calls.empty_queue"),
                                );
                                return Err(msg);
                            }

                            let executed_calls = tools::execute_tool_calls_with_app(
                                app,
                                conversation_id,
                                call_requests,
                            )
                            .await;

                            for executed in executed_calls {
                                let serialized_input =
                                    serde_json::to_string_pretty(&executed.input)
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
                                        token_usage: None,
                                        stop_reason: None,
                                        turn_state: Some("tool_completed".into()),
                                        conversation_id: conversation_id.map(str::to_string),
                                    },
                                )
                                .ok();

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
                                    prevent_continuation = true;
                                    if hook_stop_reason.is_none() {
                                        hook_stop_reason = executed.stop_reason;
                                    }
                                }
                            }
                        } else if finish_reason == "stop" {
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
        if !sse_buffer.iter().all(u8::is_ascii_whitespace) {
            let preview = String::from_utf8_lossy(&sse_buffer);
            let msg = format!(
                "OpenAI stream ended with an incomplete SSE event still buffered. Pending bytes={}, preview={}. {}",
                sse_buffer.len(),
                truncate_for_log(&preview, 800),
                format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
            );
            emit_backend_error(
                app,
                "llm.providers.openai",
                msg.clone(),
                Some("stream.incomplete_event"),
            );
            return Err(msg);
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

        let output_blocks_empty = output_blocks.is_empty();
        let tool_result_blocks_empty = tool_result_blocks.is_empty();

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
        let additional_context_messages_len = additional_context_messages.len();
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

        if output_blocks_empty && tool_result_blocks_empty {
            let msg = format!(
                "OpenAI provider is returning an empty assistant message. final_stop_reason={:?}, emitted_stop={}, input_tokens={:?}, output_tokens={:?}, prevent_continuation={}, additional_context_messages={}, {}",
                final_stop_reason,
                emitted_stop,
                current_input_tokens,
                current_output_tokens,
                prevent_continuation,
                additional_context_messages_len,
                format_stream_diagnostics(&recent_sse_events, sse_buffer.len())
            );
            emit_backend_error(
                app,
                "llm.providers.openai",
                msg.clone(),
                Some("stream.empty_assistant"),
            );
            return Err(msg);
        }

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
