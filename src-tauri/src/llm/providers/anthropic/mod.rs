pub(crate) mod prompt;
pub(crate) mod types;

use reqwest::Client;
use tauri::AppHandle;

use crate::llm::providers::stream_runner::{run_streaming, Delta, ReadyToolCall, StreamParser};
use crate::llm::providers::{ProviderTurnError, ProviderTurnResult};
use crate::llm::tools;
use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::error_event::emit_backend_error;

use super::sse_utils::truncate_for_log;
use types::{StreamContentBlock, StreamDelta, StreamEvent};

pub struct AnthropicProvider;

struct AnthropicStreamParser {
    current_tool_id: Option<String>,
    current_tool_name: Option<String>,
    current_tool_input: String,
    current_thinking: String,
    current_sig: String,
    pending_tool_calls: Vec<tools::ToolCallRequest>,
    pending_stop_reason: Option<String>,
    streaming_batch_size: usize,
}

impl AnthropicStreamParser {
    fn new() -> Self {
        let streaming_batch_size = std::env::var("NOVA_STREAMING_TOOL_BATCH_SIZE")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(2);
        Self {
            current_tool_id: None,
            current_tool_name: None,
            current_tool_input: String::new(),
            current_thinking: String::new(),
            current_sig: String::new(),
            pending_tool_calls: Vec::new(),
            pending_stop_reason: None,
            streaming_batch_size,
        }
    }
}

impl StreamParser for AnthropicStreamParser {
    fn provider_name(&self) -> &'static str {
        "anthropic"
    }

    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String> {
        let event = match serde_json::from_str::<StreamEvent>(data) {
            Ok(e) => e,
            Err(e) => {
                return Err(format!(
                    "Failed to parse Anthropic stream event: {}. Data preview: {}",
                    e,
                    truncate_for_log(data, 1200)
                ));
            }
        };

        let mut deltas: Vec<Delta> = Vec::new();

        match event {
            StreamEvent::MessageStart { message } => {
                deltas.push(Delta::Usage {
                    input: Some(message.usage.input_tokens),
                    output: Some(message.usage.output_tokens),
                });
            }

            StreamEvent::ContentBlockStart { content_block, .. } => match content_block {
                StreamContentBlock::ToolUse { id, name, .. } => {
                    self.current_tool_id = Some(id.clone());
                    self.current_tool_name = Some(name.clone());
                    self.current_tool_input.clear();
                    deltas.push(Delta::ToolStart { id: Some(id), name });
                }
                StreamContentBlock::Thinking { .. } => {
                    self.current_thinking.clear();
                    self.current_sig.clear();
                }
                StreamContentBlock::Text { .. } => {}
            },

            StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                StreamDelta::TextDelta { text } => {
                    deltas.push(Delta::Text(text));
                }
                StreamDelta::ThinkingDelta { thinking } => {
                    self.current_thinking.push_str(&thinking);
                    deltas.push(Delta::Reasoning(thinking));
                }
                StreamDelta::SignatureDelta { signature } => {
                    self.current_sig.push_str(&signature);
                }
                StreamDelta::InputJsonDelta { partial_json } => {
                    self.current_tool_input.push_str(&partial_json);
                    deltas.push(Delta::ToolArgsDelta {
                        id: self.current_tool_id.clone(),
                        args: partial_json,
                    });
                }
            },

            StreamEvent::ContentBlockStop { .. } => {
                if let (Some(id), Some(name)) =
                    (self.current_tool_id.take(), self.current_tool_name.take())
                {
                    let input_value: serde_json::Value = match serde_json::from_str(
                        &self.current_tool_input,
                    ) {
                        Ok(v) => v,
                        Err(e) => {
                            return Err(format!(
                                    "Failed to parse Anthropic tool input for '{}': {}. Raw input preview: {}",
                                    name,
                                    e,
                                    truncate_for_log(&self.current_tool_input, 1200)
                                ));
                        }
                    };
                    self.current_tool_input.clear();

                    self.pending_tool_calls.push(tools::ToolCallRequest {
                        id: id.clone(),
                        name: name.clone(),
                        input: input_value,
                    });

                    if self.pending_tool_calls.len() >= self.streaming_batch_size {
                        let batch: Vec<ReadyToolCall> =
                            std::mem::take(&mut self.pending_tool_calls)
                                .into_iter()
                                .map(|r| ReadyToolCall {
                                    id: r.id,
                                    name: r.name,
                                    input: r.input,
                                })
                                .collect();
                        deltas.push(Delta::ToolsReady(batch));
                    }
                } else if !self.current_thinking.is_empty() {
                    deltas.push(Delta::ThinkingBlock {
                        thinking: std::mem::take(&mut self.current_thinking),
                        signature: std::mem::take(&mut self.current_sig),
                    });
                }
            }

            StreamEvent::MessageDelta { delta, usage } => {
                if let Some(reason) = delta.stop_reason {
                    self.pending_stop_reason = Some(reason);
                }
                if usage.output_tokens > 0 {
                    deltas.push(Delta::Usage {
                        input: if usage.input_tokens > 0 {
                            Some(usage.input_tokens)
                        } else {
                            None
                        },
                        output: Some(usage.output_tokens),
                    });
                }
            }

            StreamEvent::MessageStop => {
                if !self.pending_tool_calls.is_empty() {
                    let batch: Vec<ReadyToolCall> = std::mem::take(&mut self.pending_tool_calls)
                        .into_iter()
                        .map(|r| ReadyToolCall {
                            id: r.id,
                            name: r.name,
                            input: r.input,
                        })
                        .collect();
                    deltas.push(Delta::ToolsReady(batch));
                }
                deltas.push(Delta::Stop {
                    reason: self.pending_stop_reason.take(),
                });
            }

            StreamEvent::Error { error } => {
                let error_type = error
                    .error_type
                    .unwrap_or_else(|| "unknown_error".to_string());
                let message = error
                    .message
                    .unwrap_or_else(|| "Anthropic stream returned an error".to_string());
                return Err(format!(
                    "Anthropic stream error [{}]: {}",
                    error_type, message
                ));
            }

            StreamEvent::Ping => {}
        }

        Ok(deltas)
    }
}

impl AnthropicProvider {
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
        let api_key = profile.api_key;

        if api_key.is_empty() {
            return Err(ProviderTurnError::new(
                "API error: No API key configured. Please set it in Settings.".to_string(),
            ));
        }

        let request = prompt::build_request(app, messages, agent_mode, conversation_id)?.request;

        let client = Client::new();
        let mut url = profile.base_url.trim_end_matches('/').to_string();
        if !url.ends_with("/v1/messages") && !url.ends_with("/messages") {
            if url.ends_with("/v1") {
                url = format!("{}/messages", url);
            } else {
                url = format!("{}/v1/messages", url);
            }
        }

        if let Ok(wire) = serde_json::to_string(&request) {
            crate::llm::utils::turn_log::log_wire_request(app, conversation_id, &url, &wire);
        }

        let resp = tokio::select! {
            res = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send() => res,
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
                    eprintln!("API Error: {}", error_text);
                    let msg = format!("API Error [{}] {} => {}", status, url, error_text);
                    emit_backend_error(
                        app,
                        "llm.providers.anthropic",
                        msg.clone(),
                        Some("http.non_success"),
                    );
                    return Err(ProviderTurnError::new(msg));
                }

                let mut parser = AnthropicStreamParser::new();
                run_streaming(&mut parser, app, res, conversation_id).await
            }
            Err(e) => {
                let msg = e.to_string();
                emit_backend_error(
                    app,
                    "llm.providers.anthropic",
                    msg.clone(),
                    Some("http.request"),
                );
                Err(ProviderTurnError::new(msg))
            }
        }
    }
}
