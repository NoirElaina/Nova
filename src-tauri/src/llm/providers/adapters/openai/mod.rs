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
    // 本轮已成功产出过工具调用（ToolsReady）。
    // OpenRouter 等网关会在内容块与收尾计费块上重复发送 finish_reason=tool_calls，
    // 第二次到达时 pending 已排空，靠此标记识别"重复收尾"而非"空流"。
    tool_calls_emitted: bool,
}

impl OpenAiAdapter {
    pub fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            inline_think: InlineThinkExtractor::default(),
            tool_calls_emitted: false,
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
    ) -> Result<(RequestBuilder, ProviderPromptEstimate), String> {
        let settings =
            crate::command::settings::get_settings(app.clone()).map_err(|e| e.to_string())?;
        let profile = settings.active_provider_profile();

        let built = prompt::build_request(app, messages, agent_mode, conversation_id)
            .map_err(|e| e.message)?;

        builder = builder.header("content-type", "application/json");

        if !profile.api_key.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", profile.api_key));
        }

        Ok((builder.json(&built.request), built.estimate))
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
            // 非增量形态：部分 OpenRouter 上游/中转后端不流式推工具调用碎片，
            // 而是在单个 chunk 的 message 里一次性给出完整消息与工具调用。
            // 必须在 finish_reason 处理之前入库，否则收尾即报“无待处理增量”。
            if let Some(message) = choice.message {
                if let Some(text) = message.content {
                    if !text.is_empty() {
                        push_inline_parts(&mut deltas, self.inline_think.push(&text));
                    }
                }
                if let Some(message_tool_calls) = message.tool_calls {
                    for (position, tc) in message_tool_calls.into_iter().enumerate() {
                        let entry = self.pending.entry(position).or_default();

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
            }

            if let Some(OpenAiDelta {
                content,
                refusal,
                reasoning_content,
                reasoning_details,
                reasoning,
                thinking_content,
                tool_calls,
                ..
            }) = choice.delta
            {
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
                        // 缺 index 的提供商（通常一次只发一个工具调用）回落到当前序号。
                        let index = tc.index.unwrap_or_else(|| self.pending.len());
                        let entry = self.pending.entry(index).or_default();

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
            }

            if let Some(finish_reason) = choice.finish_reason {
                match finish_reason.as_str() {
                    "tool_calls" => {
                        let drained: Vec<(usize, PendingToolCall)> =
                            std::mem::take(&mut self.pending).into_iter().collect();

                        if drained.is_empty() {
                            // 重复的 finish_reason=tool_calls（网关在收尾计费块上再发一次）：
                            // 工具调用已在先前的块里产出过，静默跳过。
                            if self.tool_calls_emitted {
                                continue;
                            }
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
                        self.tool_calls_emitted = true;
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
            self.tool_calls_emitted = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_ready(deltas: Vec<Delta>) -> Vec<ReadyToolCall> {
        deltas.into_iter().find_map(|d| match d {
            Delta::ToolsReady(ready) => Some(ready),
            _ => None,
        }).unwrap_or_default()
    }

    // 非增量形态：完整工具调用随 message 一次性给出（部分 OpenRouter 上游）。
    #[test]
    fn parses_non_incremental_message_tool_calls() {
        let mut adapter = OpenAiAdapter::new();
        let data = r#"{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"/tmp/a.txt\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let deltas = adapter.parse_event(data).expect("parse non-incremental chunk");
        let ready = collect_ready(deltas);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "call_1");
        assert_eq!(ready[0].name, "Read");
        assert_eq!(ready[0].input.get("file_path").and_then(|v| v.as_str()), Some("/tmp/a.txt"));
    }

    // 增量形态：多个 delta 碎片拼接（OpenAI 官方风格），回归保障。
    #[test]
    fn parses_incremental_tool_call_deltas() {
        let mut adapter = OpenAiAdapter::new();
        let first = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"Grep"}}]}}]}"#;
        let second = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"pattern\":\"x\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let _ = adapter.parse_event(first).expect("parse name chunk");
        let deltas = adapter.parse_event(second).expect("parse args chunk");
        let ready = collect_ready(deltas);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].name, "Grep");
        assert_eq!(ready[0].input.get("pattern").and_then(|v| v.as_str()), Some("x"));
    }

    // 收尾 chunk 只带 usage 不带 delta/choices 内容：不得解析失败。
    #[test]
    fn parses_usage_only_chunk_without_delta() {
        let mut adapter = OpenAiAdapter::new();
        let data = r#"{"choices":[{"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let deltas = adapter.parse_event(data).expect("parse usage-only chunk");
        assert!(deltas.iter().any(|d| matches!(d, Delta::Usage { .. })));
        assert!(deltas.iter().any(|d| matches!(d, Delta::Stop { .. })));
    }

    // 回归：OpenRouter（StreamLake 上游，deepseek 系）真实流形态——
    // reasoning 增量 + 标准增量式工具调用，且 finish_reason=tool_calls 在
    // 内容块与收尾计费块上重复发送两次。旧解析器在第二次误报"无待处理增量"
    // 终止已成功的回合；修复后必须幂等跳过。报文取自生产抓包。
    #[test]
    fn replay_openrouter_duplicate_finish_reason_stream() {
        let events = [
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"deepseek/deepseek-v4-pro","provider":"StreamLake","choices":[{"index":0,"delta":{"content":"","role":"assistant","reasoning":"The","reasoning_details":[{"type":"reasoning.text","text":"The","format":"unknown","index":0}]},"finish_reason":null,"native_finish_reason":null}]}"#,
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"deepseek/deepseek-v4-pro","provider":"StreamLake","choices":[{"index":0,"delta":{"content":"","role":"assistant","reasoning":" user wants","reasoning_details":[{"type":"reasoning.text","text":" user wants","format":"unknown","index":0}]},"finish_reason":null,"native_finish_reason":null}]}"#,
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"deepseek/deepseek-v4-pro","provider":"StreamLake","choices":[{"index":0,"delta":{"content":null,"role":"assistant","tool_calls":[{"index":0,"id":"call_76c41e51ecce48ccb7acc247","type":"function","function":{"name":"Bash","arguments":""}}]},"finish_reason":null,"native_finish_reason":null}]}"#,
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"deepseek/deepseek-v4-pro","provider":"StreamLake","choices":[{"index":0,"delta":{"content":null,"role":"assistant","tool_calls":[{"index":0,"function":{"arguments":"{\"command\": \"pwd\", \"description\": \"Show current working directory\"}"}}]},"finish_reason":null,"native_finish_reason":null}]}"#,
            // 第一次 finish_reason=tool_calls：排空 pending，产出工具调用。
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"deepseek/deepseek-v4-pro","provider":"StreamLake","choices":[{"index":0,"delta":{"content":"","role":"assistant","reasoning":null},"finish_reason":"tool_calls","native_finish_reason":"tool_calls"}]}"#,
            // 第二次 finish_reason=tool_calls（收尾计费块重复发送）：必须幂等跳过而非报错。
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"deepseek/deepseek-v4-pro","provider":"StreamLake","service_tier":null,"choices":[{"index":0,"delta":{"content":"","role":"assistant"},"finish_reason":"tool_calls","native_finish_reason":"tool_calls"}],"usage":{"prompt_tokens":9823,"completion_tokens":75,"total_tokens":9898,"prompt_tokens_details":{"cached_tokens":7759}}}"#,
        ];

        let mut adapter = OpenAiAdapter::new();
        let mut all_ready: Vec<ReadyToolCall> = Vec::new();
        let mut usage_seen = false;
        for (i, event) in events.iter().enumerate() {
            match adapter.parse_event(event) {
                Ok(deltas) => {
                    for d in deltas {
                        match d {
                            Delta::ToolsReady(ready) => all_ready.extend(ready),
                            Delta::Usage { .. } => usage_seen = true,
                            _ => {}
                        }
                    }
                }
                Err(e) => panic!("event {} failed: {}", i, e),
            }
        }
        // 工具调用恰好产出一次（重复收尾不会重复产出）。
        assert_eq!(all_ready.len(), 1, "Bash 工具调用应被完整捕获且只产出一次");
        assert_eq!(all_ready[0].name, "Bash");
        assert_eq!(all_ready[0].id, "call_76c41e51ecce48ccb7acc247");
        assert_eq!(
            all_ready[0].input.get("command").and_then(|v| v.as_str()),
            Some("pwd")
        );
        // 收尾计费块的 usage 仍正常入账。
        assert!(usage_seen);
    }

    // 真正的坏流：全程无任何工具调用却以 finish_reason=tool_calls 结束，仍需报错。
    #[test]
    fn empty_stream_with_tool_calls_finish_still_errors() {
        let mut adapter = OpenAiAdapter::new();
        let data = r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":"tool_calls"}]}"#;
        let err = adapter.parse_event(data).unwrap_err();
        assert!(err.contains("no pending tool call deltas"));
    }

    // delta 工具调用缺 index（少数提供商）：回落到当前序号，不报错。
    #[test]
    fn parses_tool_call_delta_without_index() {
        let mut adapter = OpenAiAdapter::new();
        let data = r#"{"choices":[{"delta":{"tool_calls":[{"id":"c9","function":{"name":"Glob","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#;
        let deltas = adapter.parse_event(data).expect("parse index-less chunk");
        let ready = collect_ready(deltas);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].name, "Glob");
    }
}
