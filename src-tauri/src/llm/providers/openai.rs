use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use tauri::AppHandle;

use crate::llm::providers::stream_runner::{run_streaming, Delta, ReadyToolCall, StreamParser};
use crate::llm::providers::{ProviderTurnError, ProviderTurnResult};
use crate::llm::tools;
use crate::llm::types::{AgentMode, ContentBlock, Message, Role};
use crate::llm::utils::error_event::emit_backend_error;
use crate::llm::utils::system_prompt::load_system_prompt;

use super::sse_utils::truncate_for_log;

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
        Value::Array(items) => items
            .iter()
            .map(extract_reasoning_text)
            .collect::<Vec<_>>()
            .join(""),
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

// ─────────────────────────────────────────────
// OpenAiStreamParser — 实现 StreamParser trait
// ─────────────────────────────────────────────

struct OpenAiStreamParser {
    pending: BTreeMap<usize, PendingToolCall>,
    recent_events: VecDeque<StreamDiagnosticEvent>,
    seq: u64,
}

impl OpenAiStreamParser {
    fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            recent_events: VecDeque::with_capacity(STREAM_DIAGNOSTIC_EVENT_LIMIT),
            seq: 0,
        }
    }

    fn push_diagnostic(&mut self, event: StreamDiagnosticEvent) {
        push_stream_diagnostic_event(&mut self.recent_events, event);
    }

    fn diag_ctx(&self) -> String {
        format_stream_diagnostics(&self.recent_events, 0)
    }
}

impl StreamParser for OpenAiStreamParser {
    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String> {
        self.seq += 1;

        let chunk: OpenAiStreamChunk = match serde_json::from_str(data) {
            Ok(c) => {
                let diag = build_stream_diagnostic_event(self.seq, data, Ok(&c));
                self.push_diagnostic(diag);
                c
            }
            Err(e) => {
                let err_str = e.to_string();
                let diag = build_stream_diagnostic_event(self.seq, data, Err(err_str.clone()));
                self.push_diagnostic(diag);
                return Err(format!(
                    "Failed to parse OpenAI SSE event JSON: {}. Data preview: {}. {}",
                    err_str,
                    truncate_for_log(data, 800),
                    self.diag_ctx()
                ));
            }
        };

        let mut deltas: Vec<Delta> = Vec::new();

        // Token usage（末尾 chunk 中）。
        if let Some(usage) = chunk.usage {
            let output = usage.completion_tokens.or_else(|| {
                usage
                    .total_tokens
                    .zip(usage.prompt_tokens)
                    .and_then(|(total, prompt)| total.checked_sub(prompt))
            });
            deltas.push(Delta::Usage {
                input: usage.prompt_tokens,
                output,
            });
        }

        for choice in chunk.choices {
            let OpenAiDelta {
                content,
                tool_calls,
                extra,
            } = choice.delta;

            // 文本增量。
            if let Some(text) = content {
                if !text.is_empty() {
                    deltas.push(Delta::Text(text));
                }
            }

            // 推理增量（OpenAI o 系列 / 各兼容厂商扩展字段）。
            for key in ["reasoning", "reasoning_content"] {
                if let Some(value) = extra.get(key) {
                    let text = extract_reasoning_text(value);
                    if !text.is_empty() {
                        deltas.push(Delta::Reasoning(text));
                    }
                }
            }

            // 工具调用增量：累积到 pending map 中。
            if let Some(tool_call_deltas) = tool_calls {
                for tc in tool_call_deltas {
                    let entry = self.pending.entry(tc.index).or_default();

                    if let Some(id) = tc.id {
                        entry.id = Some(id);
                    }

                    if let Some(func) = tc.function {
                        if let Some(name) = func.name {
                            if entry.name.is_none() {
                                // 首次出现工具名时通知前端。
                                deltas.push(Delta::ToolStart {
                                    id: entry.id.clone(),
                                    name: name.clone(),
                                });
                            }
                            entry.name = Some(name);
                        }
                        if let Some(args) = func.arguments {
                            deltas.push(Delta::ToolArgsDelta {
                                id: entry.id.clone(),
                                args: args.clone(),
                            });
                            entry.arguments.push_str(&args);
                        }
                    }
                }
            }

            // finish_reason 驱动工具执行。
            if let Some(finish_reason) = choice.finish_reason {
                if finish_reason == "tool_calls" {
                    let drained: Vec<(usize, PendingToolCall)> =
                        std::mem::take(&mut self.pending).into_iter().collect();

                    if drained.is_empty() {
                        return Err(format!(
                            "OpenAI stream reported finish_reason=tool_calls but no pending tool call deltas were captured. {}",
                            self.diag_ctx()
                        ));
                    }

                    let mut ready: Vec<ReadyToolCall> = Vec::new();
                    for (index, tc) in drained {
                        let (id, name) = match (tc.id, tc.name) {
                            (Some(id), Some(name)) => (id, name),
                            (id, name) => {
                                return Err(format!(
                                    "OpenAI tool call at index={} incomplete at finish_reason=tool_calls: has_id={:?}, has_name={:?}, args_preview={}. {}",
                                    index, id, name,
                                    truncate_for_log(&tc.arguments, 800),
                                    self.diag_ctx()
                                ));
                            }
                        };
                        let input: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                            Ok(v) => v,
                            Err(e) => {
                                return Err(format!(
                                    "Failed to parse OpenAI tool call arguments for '{}': {}. Args preview: {}. {}",
                                    name, e,
                                    truncate_for_log(&tc.arguments, 800),
                                    self.diag_ctx()
                                ));
                            }
                        };
                        ready.push(ReadyToolCall { id, name, input });
                    }
                    deltas.push(Delta::ToolsReady(ready));
                } else if finish_reason == "stop" {
                    deltas.push(Delta::Stop {
                        reason: Some(finish_reason),
                    });
                }
            }
        }

        Ok(deltas)
    }

    fn flush(&mut self) -> Vec<Delta> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let drained: Vec<(usize, PendingToolCall)> =
            std::mem::take(&mut self.pending).into_iter().collect();
        let mut ready: Vec<ReadyToolCall> = Vec::new();
        for (_index, tc) in drained {
            if let (Some(id), Some(name)) = (tc.id, tc.name) {
                if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tc.arguments) {
                    ready.push(ReadyToolCall { id, name, input });
                }
            }
        }
        if ready.is_empty() {
            Vec::new()
        } else {
            vec![Delta::ToolsReady(ready)]
        }
    }
}

pub struct OpenAiProvider;

impl OpenAiProvider {
    pub async fn send_request(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, ProviderTurnError> {
        // 读取设置并拿到当前 provider profile。
        let settings =
            crate::command::settings::get_settings(app.clone()).map_err(ProviderTurnError::new)?;
        let profile = settings.active_provider_profile();

        // 仅注入内置工具；MCP 采用 server 级发现，避免每轮发送全部动态工具 schema。
        let available_tools = tools::get_available_tools();

        // 加载系统提示词（含 Agent/Plan/Auto 模式逻辑）。
        let system_prompt = load_system_prompt(app, agent_mode, conversation_id)?;

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
                                        return Err(ProviderTurnError::new(msg));
                                    }
                                };
                                // 组装 assistant.tool_calls 条目。
                                tool_calls.push(OpenAiReqToolCall {
                                    id: id.clone(),
                                    r#type: "function".into(),
                                    function: OpenAiReqFunction {
                                        name: name.clone(),
                                        arguments: serialized_args,
                                    },
                                });
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                is_error: _,
                                content,
                            } => {
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
                        let tc = if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        };
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
            req_builder =
                req_builder.header("Authorization", format!("Bearer {}", profile.api_key));
        }

        // @@日志记录 wire_request — 记录实际发出的 HTTP 请求 JSON。
        if let Ok(wire) = serde_json::to_string(&request) {
            crate::llm::utils::turn_log::log_wire_request(app, conversation_id, &url, &wire);
        }

        // 发起请求；tokio::select! 竞争取消轮询，避免卡在 DNS/TLS/建连阶段无法响应取消。
        let resp = tokio::select! {
            res = req_builder.json(&request).send() => res,
            _ = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    if crate::llm::cancellation::is_cancelled(conversation_id) { break; }
                }
            } => {
                return Ok(ProviderTurnResult {
                    messages: Vec::new(),
                    stop_reason: Some("cancelled".into()),
                    input_tokens: None,
                    output_tokens: None,
                    prevent_continuation: false,
                });
            }
        };

        // 处理 HTTP 结果：成功走流式解析，失败返回错误。
        match resp {
            Ok(res) => {
                if !res.status().is_success() {
                    // 非 2xx 时读取响应文本并上报。
                    let status = res.status();
                    let error_text = res.text().await.unwrap_or_default();
                    eprintln!("API Error: {}", error_text);
                    let msg = format!("API Error [{}] {} => {}", status, url, error_text);
                    emit_backend_error(
                        app,
                        "llm.providers.openai",
                        msg.clone(),
                        Some("http.non_success"),
                    );
                    return Err(ProviderTurnError::new(msg));
                }

                {
                    let mut parser = OpenAiStreamParser::new();
                    run_streaming(&mut parser, app, res, conversation_id).await
                }
            }
            Err(e) => {
                let msg = e.to_string();
                emit_backend_error(
                    app,
                    "llm.providers.openai",
                    msg.clone(),
                    Some("http.request"),
                );
                Err(ProviderTurnError::new(msg))
            }
        }
    }
}
