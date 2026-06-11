pub(crate) mod prompt;
pub(crate) mod types;

use serde_json::Value;
use std::collections::BTreeMap;
use reqwest::RequestBuilder;

use crate::llm::providers::adapters::ApiAdapter;
use crate::llm::providers::stream_runner::{Delta, ReadyToolCall};
use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::types::{AgentMode, Message};

use super::super::sse_utils::truncate_for_log;
use types::{
    ResponsesOutputItem, ResponsesReasoningSummaryPart, ResponsesResponse, ResponsesStreamEvent,
};

#[derive(Debug, Default)]
struct PendingFunctionCall {
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Default)]
struct PendingReasoning {
    summary: String,
}

pub struct ResponsesAdapter {
    pending_fn_calls: BTreeMap<usize, PendingFunctionCall>,
    pending_reasoning: BTreeMap<usize, PendingReasoning>,
}

impl ResponsesAdapter {
    pub fn new() -> Self {
        Self {
            pending_fn_calls: BTreeMap::new(),
            pending_reasoning: BTreeMap::new(),
        }
    }
}

fn response_error_message(prefix: &str, response: ResponsesResponse) -> String {
    let response_id = response.id.unwrap_or_else(|| "unknown".to_string());
    let status = response.status.unwrap_or_else(|| "unknown".to_string());

    if let Some(error) = response.error {
        let code = error.code.unwrap_or_else(|| "unknown".to_string());
        let message = error.message.unwrap_or_else(|| "unknown error".to_string());
        return format!(
            "{}: id={}, status={}, code={}, message={}",
            prefix, response_id, status, code, message
        );
    }

    if let Some(details) = response.incomplete_details {
        let reason = details.reason.unwrap_or_else(|| "unknown".to_string());
        return format!(
            "{}: id={}, status={}, reason={}",
            prefix, response_id, status, reason
        );
    }

    format!("{}: id={}, status={}", prefix, response_id, status)
}

fn ready_tool_call(
    output_index: usize,
    item: ResponsesOutputItem,
    pending: Option<PendingFunctionCall>,
) -> Result<Option<ReadyToolCall>, String> {
    if item.item_type != "function_call" {
        return Ok(None);
    }

    let call_id = item
        .call_id
        .or_else(|| pending.as_ref().and_then(|p| p.call_id.clone()));
    let name = item
        .name
        .or_else(|| pending.as_ref().and_then(|p| p.name.clone()));
    let arguments = item
        .arguments
        .or_else(|| pending.as_ref().map(|p| p.arguments.clone()))
        .unwrap_or_default();

    let (call_id, name) = match (call_id, name) {
        (Some(id), Some(name)) => (id, name),
        (id, name) => {
            return Err(format!(
                "Responses API function_call at output_index={} missing call_id or name: has_id={}, has_name={}",
                output_index,
                id.is_some(),
                name.is_some()
            ));
        }
    };

    let input: Value = serde_json::from_str(&arguments).map_err(|e| {
        format!(
            "Failed to parse Responses API function call arguments for '{}': {}. Args: {}",
            name,
            e,
            truncate_for_log(&arguments, 800)
        )
    })?;

    Ok(Some(ReadyToolCall {
        id: call_id,
        name,
        input,
    }))
}

fn reasoning_summary_to_text(summary: Option<Vec<ResponsesReasoningSummaryPart>>) -> String {
    summary
        .unwrap_or_default()
        .into_iter()
        .map(|part| match part {
            ResponsesReasoningSummaryPart::SummaryText { text } => text,
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn completed_stop_reason(response: ResponsesResponse) -> (Option<types::ResponsesUsage>, String) {
    let status = response.status.unwrap_or_else(|| "completed".to_string());
    (response.usage, format!("responses.{}", status))
}

impl ApiAdapter for ResponsesAdapter {
    fn provider_name(&self) -> &'static str {
        "responses"
    }

    fn build_request(
        &mut self,
        mut builder: RequestBuilder,
        app: &tauri::AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<RequestBuilder, String> {
        let settings = crate::command::settings::get_settings(app.clone()).map_err(|e| e.to_string())?;
        let profile = settings.active_provider_profile();

        let request = prompt::build_request(app, messages, agent_mode, conversation_id).map_err(|e| e.message)?.request;

        builder = builder.header("content-type", "application/json");

        if !profile.api_key.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", profile.api_key));
        }

        Ok(builder.json(&request))
    }

    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String> {
        if data == "[DONE]" {
            return Ok(Vec::new());
        }

        let event: ResponsesStreamEvent = serde_json::from_str(data).map_err(|e| {
            format!(
                "Failed to parse Responses API SSE event: {}. Data: {}",
                e,
                truncate_for_log(data, 1200)
            )
        })?;

        let mut deltas: Vec<Delta> = Vec::new();

        match event {
            ResponsesStreamEvent::Created { .. }
            | ResponsesStreamEvent::Queued { .. }
            | ResponsesStreamEvent::InProgress { .. } => {}

            ResponsesStreamEvent::OutputItemAdded { output_index, item } => {
                if item.item_type == "function_call" {
                    let entry = self.pending_fn_calls.entry(output_index).or_default();
                    entry.call_id = item.call_id.clone();
                    entry.name = item.name.clone();
                    if let Some(name) = item.name {
                        deltas.push(Delta::ToolStart {
                            id: item.call_id,
                            name,
                        });
                    }
                } else if item.item_type == "reasoning" {
                    let entry = self.pending_reasoning.entry(output_index).or_default();
                    let text = reasoning_summary_to_text(item.summary);
                    if !text.is_empty() {
                        entry.summary.push_str(&text);
                    }
                }
            }

            ResponsesStreamEvent::ContentPartAdded { .. }
            | ResponsesStreamEvent::ContentPartDone { .. }
            | ResponsesStreamEvent::OutputTextDone { .. } => {}

            ResponsesStreamEvent::OutputTextDelta { delta } => {
                deltas.push(Delta::Text(delta));
            }

            ResponsesStreamEvent::RefusalDelta { delta } => {
                deltas.push(Delta::Text(delta));
            }

            ResponsesStreamEvent::RefusalDone { .. } => {}

            ResponsesStreamEvent::ReasoningSummaryPartAdded { output_index, .. } => {
                self.pending_reasoning.entry(output_index).or_default();
            }

            ResponsesStreamEvent::ReasoningSummaryPartDone {
                output_index, part, ..
            } => {
                let text = reasoning_summary_to_text(part.map(|part| vec![part]));
                if !text.is_empty() {
                    let entry = self.pending_reasoning.entry(output_index).or_default();
                    if entry.summary.is_empty() {
                        entry.summary = text;
                    }
                }
            }

            ResponsesStreamEvent::ReasoningSummaryTextDelta {
                output_index,
                delta,
                ..
            } => {
                if !delta.is_empty() {
                    let entry = self.pending_reasoning.entry(output_index).or_default();
                    entry.summary.push_str(&delta);
                    deltas.push(Delta::Reasoning(delta));
                }
            }

            ResponsesStreamEvent::ReasoningSummaryTextDone {
                output_index, text, ..
            } => {
                if !text.is_empty() {
                    let entry = self.pending_reasoning.entry(output_index).or_default();
                    if entry.summary.is_empty() {
                        entry.summary = text.clone();
                        deltas.push(Delta::Reasoning(text));
                    }
                }
            }

            ResponsesStreamEvent::FunctionCallArgumentsDelta {
                output_index,
                delta,
            } => {
                let entry = self.pending_fn_calls.entry(output_index).or_default();
                entry.arguments.push_str(&delta);
                deltas.push(Delta::ToolArgsDelta {
                    id: entry.call_id.clone(),
                    args: delta,
                });
            }

            ResponsesStreamEvent::FunctionCallArgumentsDone {
                output_index,
                arguments,
            } => {
                let entry = self.pending_fn_calls.entry(output_index).or_default();
                entry.arguments = arguments;
            }

            ResponsesStreamEvent::OutputItemDone { output_index, item } => {
                if item.item_type == "reasoning" {
                    let pending = self.pending_reasoning.remove(&output_index);
                    let mut thinking = pending.map(|entry| entry.summary).unwrap_or_default();
                    if thinking.trim().is_empty() {
                        thinking = reasoning_summary_to_text(item.summary);
                    }
                    if !thinking.trim().is_empty() {
                        deltas.push(Delta::ThinkingBlock {
                            thinking,
                            signature: String::new(),
                        });
                    }
                    return Ok(deltas);
                }

                let pending = self.pending_fn_calls.remove(&output_index);
                if let Some(ready) = ready_tool_call(output_index, item, pending)? {
                    deltas.push(Delta::ToolsReady(vec![ready]));
                }
            }

            ResponsesStreamEvent::Completed { response } => {
                let (usage, status) = completed_stop_reason(response);
                if let Some(usage) = usage {
                    let cache_read = usage.cache_read_input_tokens.or_else(|| {
                        usage
                            .input_tokens_details
                            .and_then(|details| details.cached_tokens)
                    });
                    deltas.push(Delta::Usage {
                        input: usage.input_tokens,
                        output: usage.output_tokens,
                        cache_read,
                        cache_creation: usage.cache_creation_input_tokens,
                    });
                }
                deltas.push(Delta::Stop {
                    reason: Some(status),
                });
            }

            ResponsesStreamEvent::Failed { response } => {
                return Err(response_error_message(
                    "Responses API response.failed",
                    response,
                ));
            }

            ResponsesStreamEvent::Incomplete { response } => {
                return Err(response_error_message(
                    "Responses API response.incomplete",
                    response,
                ));
            }

            ResponsesStreamEvent::OutputItemFailed { output_index, item } => {
                let item_type = item
                    .map(|value| value.item_type)
                    .unwrap_or_else(|| "unknown".to_string());
                return Err(format!(
                    "Responses API output item failed at output_index={}: item_type={}",
                    output_index, item_type
                ));
            }

            ResponsesStreamEvent::Error { code, message } => {
                return Err(format!(
                    "Responses API stream error: code={}, message={}",
                    code.unwrap_or_else(|| "unknown".to_string()),
                    message.unwrap_or_else(|| "unknown error".to_string())
                ));
            }
        }

        Ok(deltas)
    }
}

pub fn estimate_prompt_tokens(
    app: &tauri::AppHandle,
    messages: &[Message],
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<ProviderPromptEstimate, ProviderTurnError> {
    prompt::build_request(app, messages, agent_mode, conversation_id)
        .map(|built| built.estimate)
}
