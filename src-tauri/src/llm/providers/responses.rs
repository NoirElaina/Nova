use reqwest::Client;
use serde_json::Value;
use std::collections::BTreeMap;
use tauri::AppHandle;

use crate::llm::providers::stream_runner::{run_streaming, Delta, ReadyToolCall, StreamParser};
use crate::llm::providers::{ProviderTurnError, ProviderTurnResult};
use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::error_event::emit_backend_error;

use super::sse_utils::truncate_for_log;

// OpenAI Responses API Provider。
// 端点: POST /v1/responses
// 与 Chat Completions 的主要区别：
//   - 消息数组字段名 messages → input
//   - system prompt 通过顶层 instructions 字段传递
//   - 工具结果格式: type=function_call_output + call_id（不是 tool_call_id）
//   - 工具调用格式: type=function_call + call_id
//   - SSE 事件类型: response.output_text.delta / response.function_call_arguments.delta 等
//   - usage 在 response.completed 事件里，字段名 input_tokens / output_tokens

pub struct ResponsesProvider;

// 流内正在累积的 function call 状态（按 output_index 索引）。
#[derive(Debug, Default)]
struct PendingFunctionCall {
    // 调用 ID（用于关联工具结果）。
    call_id: Option<String>,
    // 工具函数名。
    name: Option<String>,
    // 累积中的 JSON 参数字符串。
    arguments: String,
}

// ─────────────────────────────────────────────
// ResponsesStreamParser — 实现 StreamParser trait
// ─────────────────────────────────────────────

struct ResponsesStreamParser {
    pending_fn_calls: BTreeMap<usize, PendingFunctionCall>,
}

impl ResponsesStreamParser {
    fn new() -> Self {
        Self {
            pending_fn_calls: BTreeMap::new(),
        }
    }
}

impl StreamParser for ResponsesStreamParser {
    fn provider_name(&self) -> &'static str {
        "responses"
    }

    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String> {
        if data == "[DONE]" {
            return Ok(Vec::new());
        }

        let event: Value = serde_json::from_str(data).map_err(|e| {
            format!(
                "Failed to parse Responses API SSE event: {}. Data: {}",
                e,
                truncate_for_log(data, 1200)
            )
        })?;

        let event_type = event["type"].as_str().unwrap_or("").to_owned();
        let mut deltas: Vec<Delta> = Vec::new();

        match event_type.as_str() {
            "response.output_item.added" => {
                let output_index = event["output_index"].as_u64().unwrap_or(0) as usize;
                let item = &event["item"];
                if item["type"].as_str() == Some("function_call") {
                    let call_id = item["call_id"].as_str().map(str::to_string);
                    let name = item["name"].as_str().map(str::to_string);
                    let entry = self.pending_fn_calls.entry(output_index).or_default();
                    entry.call_id = call_id.clone();
                    entry.name = name.clone();
                    if let Some(n) = name {
                        deltas.push(Delta::ToolStart {
                            id: call_id,
                            name: n,
                        });
                    }
                }
            }

            "response.output_text.delta" => {
                if let Some(delta) = event["delta"].as_str() {
                    deltas.push(Delta::Text(delta.to_string()));
                }
            }

            "response.reasoning_summary_text.delta" => {
                if let Some(delta) = event["delta"].as_str() {
                    if !delta.is_empty() {
                        deltas.push(Delta::Reasoning(delta.to_string()));
                    }
                }
            }

            "response.function_call_arguments.delta" => {
                let output_index = event["output_index"].as_u64().unwrap_or(0) as usize;
                if let Some(delta) = event["delta"].as_str() {
                    let entry = self.pending_fn_calls.entry(output_index).or_default();
                    entry.arguments.push_str(delta);
                    deltas.push(Delta::ToolArgsDelta {
                        id: entry.call_id.clone(),
                        args: delta.to_string(),
                    });
                }
            }

            "response.output_item.done" => {
                let output_index = event["output_index"].as_u64().unwrap_or(0) as usize;
                let item = &event["item"];
                if item["type"].as_str() != Some("function_call") {
                    return Ok(Vec::new());
                }

                let call_id = item["call_id"].as_str().map(str::to_string).or_else(|| {
                    self.pending_fn_calls
                        .get(&output_index)
                        .and_then(|p| p.call_id.clone())
                });
                let name = item["name"].as_str().map(str::to_string).or_else(|| {
                    self.pending_fn_calls
                        .get(&output_index)
                        .and_then(|p| p.name.clone())
                });
                let arguments = item["arguments"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        self.pending_fn_calls
                            .get(&output_index)
                            .map(|p| p.arguments.clone())
                            .unwrap_or_default()
                    });
                self.pending_fn_calls.remove(&output_index);

                let (call_id, name) = match (call_id, name) {
                    (Some(id), Some(n)) => (id, n),
                    (id, name) => {
                        return Err(format!(
                            "Responses API function_call at output_index={} missing call_id or name: has_id={}, has_name={}",
                            output_index, id.is_some(), name.is_some()
                        ));
                    }
                };

                let input_value: Value = serde_json::from_str(&arguments).map_err(|e| {
                    format!("Failed to parse Responses API function call arguments for '{}': {}. Args: {}",
                        name, e, truncate_for_log(&arguments, 800))
                })?;

                deltas.push(Delta::ToolsReady(vec![ReadyToolCall {
                    id: call_id,
                    name,
                    input: input_value,
                }]));
            }

            "response.completed" => {
                let usage = &event["response"]["usage"];
                let input = usage["input_tokens"].as_u64().map(|v| v as u32);
                let output = usage["output_tokens"].as_u64().map(|v| v as u32);
                deltas.push(Delta::Usage { input, output });
                deltas.push(Delta::Stop {
                    reason: Some("end_turn".to_string()),
                });
            }

            "error" => {
                let code = event["code"].as_str().unwrap_or("unknown");
                let message = event["message"].as_str().unwrap_or("unknown error");
                return Err(format!(
                    "Responses API stream error: code={}, message={}",
                    code, message
                ));
            }

            _ => {}
        }

        Ok(deltas)
    }
}
impl ResponsesProvider {
    pub async fn send_request(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, ProviderTurnError> {
        let settings =
            crate::command::settings::get_settings(app.clone()).map_err(ProviderTurnError::new)?;
        let profile = settings.active_provider_profile();

        let request =
            super::responses_prompt::build_request(app, messages, agent_mode, conversation_id)?
                .request;

        let client = Client::new();

        // 规范化到 /v1/responses 端点。
        let mut url = profile.base_url.trim_end_matches('/').to_string();
        if !url.ends_with("/v1/responses") && !url.ends_with("/responses") {
            if url.ends_with("/v1") {
                url = format!("{}/responses", url);
            } else {
                url = format!("{}/v1/responses", url);
            }
        }

        let mut req_builder = client.post(&url).header("content-type", "application/json");

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

        match resp {
            Ok(res) => {
                if !res.status().is_success() {
                    let status = res.status();
                    let error_text = res.text().await.unwrap_or_default();
                    let msg = format!("API Error [{}] {} => {}", status, url, error_text);
                    emit_backend_error(
                        app,
                        "llm.providers.responses",
                        msg.clone(),
                        Some("http.non_success"),
                    );
                    return Err(ProviderTurnError::new(msg));
                }
                let mut parser = ResponsesStreamParser::new();
                run_streaming(&mut parser, app, res, conversation_id).await
            }
            Err(e) => {
                let msg = e.to_string();
                emit_backend_error(
                    app,
                    "llm.providers.responses",
                    msg.clone(),
                    Some("http.request"),
                );
                Err(ProviderTurnError::new(msg))
            }
        }
    }
}
