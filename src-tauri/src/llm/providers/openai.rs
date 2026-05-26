use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use tauri::AppHandle;

use crate::llm::providers::stream_runner::{run_streaming, Delta, ReadyToolCall, StreamParser};
use crate::llm::providers::{ProviderTurnError, ProviderTurnResult};
use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::error_event::emit_backend_error;

use super::sse_utils::truncate_for_log;

// OpenAI Provider 相关结构体定义。
// 主要负责：
// - 将 internal Message -> OpenAI JSON message
// - 触发 /v1/chat/completions?stream
// - 处理流式 SSE Delta 并 emit 到前端

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

#[derive(Debug, Default)]
struct PendingToolCall {
    // 累积到的调用 ID。
    id: Option<String>,
    // 累积到的工具名。
    name: Option<String>,
    // 累积到的 JSON 参数字符串。
    arguments: String,
}

// ─────────────────────────────────────────────
// OpenAiStreamParser — 实现 StreamParser trait
// ─────────────────────────────────────────────

struct OpenAiStreamParser {
    pending: BTreeMap<usize, PendingToolCall>,
}

impl OpenAiStreamParser {
    fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
        }
    }
}

impl StreamParser for OpenAiStreamParser {
    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String> {
        let chunk: OpenAiStreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                return Err(format!(
                    "Failed to parse OpenAI SSE event JSON: {}. Data preview: {}",
                    e,
                    truncate_for_log(data, 800)
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
                        return Err(
                            "OpenAI stream reported finish_reason=tool_calls but no pending tool call deltas were captured."
                                .to_string(),
                        );
                    }

                    let mut ready: Vec<ReadyToolCall> = Vec::new();
                    for (index, tc) in drained {
                        let (id, name) = match (tc.id, tc.name) {
                            (Some(id), Some(name)) => (id, name),
                            (id, name) => {
                                return Err(format!(
                                    "OpenAI tool call at index={} incomplete at finish_reason=tool_calls: has_id={:?}, has_name={:?}, args_preview={}",
                                    index, id, name,
                                    truncate_for_log(&tc.arguments, 800)
                                ));
                            }
                        };
                        let input: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                            Ok(v) => v,
                            Err(e) => {
                                return Err(format!(
                                    "Failed to parse OpenAI tool call arguments for '{}': {}. Args preview: {}",
                                    name, e,
                                    truncate_for_log(&tc.arguments, 800)
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

        let request =
            super::openai_prompt::build_request(app, messages, agent_mode, conversation_id)?
                .request;

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
