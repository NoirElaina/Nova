pub(crate) mod prompt;
pub(crate) mod types;

use reqwest::RequestBuilder;
use std::collections::BTreeMap;

use super::reasoning::{extract_reasoning_field_text, push_inline_parts, InlineThinkExtractor};
use crate::llm::providers::adapters::{parse_tool_arguments, ApiAdapter};
use crate::llm::providers::stream_runner::{Delta, ReadyToolCall};
use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::types::{AgentMode, Message};

use super::super::sse_utils::truncate_for_log;
use types::{OpenAiDelta, OpenAiStreamChunk};

#[derive(Debug, Default)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

pub struct OpenAiAdapter {
    pending: BTreeMap<usize, PendingToolCall>,
    inline_think: InlineThinkExtractor,
}

impl OpenAiAdapter {
    pub fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            inline_think: InlineThinkExtractor::default(),
        }
    }
}

impl ApiAdapter for OpenAiAdapter {
    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn build_request(
        &mut self,
        mut builder: RequestBuilder,
        app: &tauri::AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<RequestBuilder, String> {
        let settings =
            crate::command::settings::get_settings(app.clone()).map_err(|e| e.to_string())?;
        let profile = settings.active_provider_profile();

        let request = prompt::build_request(app, messages, agent_mode, conversation_id)
            .map_err(|e| e.message)?
            .request;

        builder = builder.header("content-type", "application/json");

        if !profile.api_key.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", profile.api_key));
        }

        Ok(builder.json(&request))
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
                cache_read: usage
                    .prompt_tokens_details
                    .and_then(|details| details.cached_tokens),
                cache_creation: None,
            });
        }

        for choice in chunk.choices {
            let OpenAiDelta {
                content,
                refusal,
                reasoning_content,
                reasoning_details,
                reasoning,
                thinking_content,
                tool_calls,
                ..
            } = choice.delta;

            let reasoning_fields = serde_json::json!({
                "reasoning_content": reasoning_content,
                "reasoning_details": reasoning_details,
                "reasoning": reasoning,
                "thinking_content": thinking_content,
            });
            if let Some(text) = extract_reasoning_field_text(&reasoning_fields) {
                deltas.push(Delta::Reasoning(text));
            }

            if let Some(text) = content {
                if !text.is_empty() {
                    push_inline_parts(&mut deltas, self.inline_think.push(&text));
                }
            }

            if let Some(text) = refusal {
                if !text.is_empty() {
                    push_inline_parts(&mut deltas, self.inline_think.push(&text));
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
                            let input = parse_tool_arguments(&name, &tc.arguments)?;
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
        let mut deltas = Vec::new();
        push_inline_parts(&mut deltas, self.inline_think.flush());

        if self.pending.is_empty() {
            return deltas;
        }
        let drained: Vec<(usize, PendingToolCall)> =
            std::mem::take(&mut self.pending).into_iter().collect();
        let mut ready: Vec<ReadyToolCall> = Vec::new();
        for (_index, tc) in drained {
            if let (Some(id), Some(name)) = (tc.id, tc.name) {
                if let Ok(input) = parse_tool_arguments(&name, &tc.arguments) {
                    ready.push(ReadyToolCall { id, name, input });
                }
            }
        }
        if !ready.is_empty() {
            deltas.push(Delta::ToolsReady(ready));
        }
        deltas
    }
}

pub fn estimate_prompt_tokens(
    app: &tauri::AppHandle,
    messages: &[Message],
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<ProviderPromptEstimate, ProviderTurnError> {
    prompt::build_request(app, messages, agent_mode, conversation_id).map(|built| built.estimate)
}
