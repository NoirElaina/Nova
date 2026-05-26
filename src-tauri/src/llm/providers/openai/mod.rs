pub(crate) mod prompt;
pub(crate) mod types;

use reqwest::Client;
use std::collections::BTreeMap;
use tauri::AppHandle;

use crate::llm::providers::stream_runner::{run_streaming, Delta, ReadyToolCall, StreamParser};
use crate::llm::providers::{ProviderTurnError, ProviderTurnResult};
use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::error_event::emit_backend_error;

use super::sse_utils::truncate_for_log;
use types::{OpenAiDelta, OpenAiStreamChunk};

#[derive(Debug, Default)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

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
                    "Failed to parse OpenAI Chat Completions SSE event JSON: {}. Data preview: {}",
                    e,
                    truncate_for_log(data, 800)
                ));
            }
        };

        let mut deltas: Vec<Delta> = Vec::new();

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
                refusal,
                tool_calls,
                ..
            } = choice.delta;

            if let Some(text) = content {
                if !text.is_empty() {
                    deltas.push(Delta::Text(text));
                }
            }

            if let Some(text) = refusal {
                if !text.is_empty() {
                    deltas.push(Delta::Text(text));
                }
            }

            if let Some(tool_call_deltas) = tool_calls {
                for tc in tool_call_deltas {
                    let entry = self.pending.entry(tc.index).or_default();

                    if let Some(id) = tc.id {
                        entry.id = Some(id);
                    }

                    if let Some(func) = tc.function {
                        if let Some(name) = func.name {
                            if entry.name.is_none() {
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

            if let Some(finish_reason) = choice.finish_reason {
                match finish_reason.as_str() {
                    "tool_calls" => {
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
                                        index,
                                        id,
                                        name,
                                        truncate_for_log(&tc.arguments, 800)
                                    ));
                                }
                            };
                            let input: serde_json::Value = match serde_json::from_str(&tc.arguments)
                            {
                                Ok(v) => v,
                                Err(e) => {
                                    return Err(format!(
                                            "Failed to parse OpenAI tool call arguments for '{}': {}. Args preview: {}",
                                            name,
                                            e,
                                            truncate_for_log(&tc.arguments, 800)
                                        ));
                                }
                            };
                            ready.push(ReadyToolCall { id, name, input });
                        }
                        deltas.push(Delta::ToolsReady(ready));
                    }
                    "stop" | "length" | "content_filter" => {
                        deltas.push(Delta::Stop {
                            reason: Some(finish_reason),
                        });
                    }
                    "function_call" => {
                        return Err(
                            "OpenAI Chat Completions returned deprecated finish_reason=function_call; Nova only supports tool_calls."
                                .to_string(),
                        );
                    }
                    _ => {
                        deltas.push(Delta::Stop {
                            reason: Some(finish_reason),
                        });
                    }
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
        let settings =
            crate::command::settings::get_settings(app.clone()).map_err(ProviderTurnError::new)?;
        let profile = settings.active_provider_profile();

        let request = prompt::build_request(app, messages, agent_mode, conversation_id)?.request;

        let client = Client::new();
        let mut url = profile.base_url.trim_end_matches('/').to_string();
        if !url.ends_with("/v1/chat/completions") && !url.ends_with("/chat/completions") {
            if url.ends_with("/v1") {
                url = format!("{}/chat/completions", url);
            } else {
                url = format!("{}/v1/chat/completions", url);
            }
        }

        let mut req_builder = client.post(&url).header("content-type", "application/json");

        if !profile.api_key.is_empty() {
            req_builder =
                req_builder.header("Authorization", format!("Bearer {}", profile.api_key));
        }

        if let Ok(wire) = serde_json::to_string(&request) {
            crate::llm::utils::turn_log::log_wire_request(app, conversation_id, &url, &wire);
        }

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

                let mut parser = OpenAiStreamParser::new();
                run_streaming(&mut parser, app, res, conversation_id).await
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
