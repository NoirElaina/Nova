use serde_json::Value;
use tauri::AppHandle;

use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::services::compact;
use crate::llm::tools;
use crate::llm::types::{AgentMode, Content, ContentBlock, ImageSource, Message, Role};
use crate::llm::utils::system_prompt::load_system_prompt;

use super::types::{
    OpenAiFunction, OpenAiMessage, OpenAiReqFunction, OpenAiReqToolCall, OpenAiRequest,
    OpenAiStreamOptions, OpenAiTool,
};

pub(crate) struct BuiltOpenAiRequest {
    pub request: OpenAiRequest,
    pub estimate: ProviderPromptEstimate,
}

fn clamp_i64_to_u32(value: i64) -> u32 {
    if value <= 0 {
        0
    } else if value >= u32::MAX as i64 {
        u32::MAX
    } else {
        value as u32
    }
}

fn build_openai_image_part(source: &ImageSource) -> Option<Value> {
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

fn messages_to_openai_messages(
    messages: &[Message],
) -> Result<Vec<OpenAiMessage>, ProviderTurnError> {
    let mut oai_messages = Vec::new();

    for message in messages {
        let base_role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        match &message.content {
            Content::Text(text) => {
                oai_messages.push(OpenAiMessage {
                    role: base_role.into(),
                    content: Some(Value::String(text.clone())),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            Content::Blocks(blocks) => {
                let mut text_parts = Vec::new();
                let mut image_parts = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();

                for block in blocks {
                    match block {
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
                            let arguments = serde_json::to_string(input).map_err(|error| {
                                ProviderTurnError::new(format!(
                                    "Failed to serialize OpenAI tool arguments for '{}': {}",
                                    name, error
                                ))
                            })?;
                            tool_calls.push(OpenAiReqToolCall {
                                id: id.clone(),
                                r#type: "function".into(),
                                function: OpenAiReqFunction {
                                    name: name.clone(),
                                    arguments,
                                },
                            });
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            let text = content
                                .iter()
                                .filter_map(|block| {
                                    if let ContentBlock::Text { text } = block {
                                        Some(text.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            tool_results.push((tool_use_id.clone(), text));
                        }
                    }
                }

                if base_role == "assistant" {
                    let content = if text_parts.is_empty() && !tool_calls.is_empty() {
                        None
                    } else {
                        Some(Value::String(text_parts.join("\n")))
                    };
                    let tool_calls = if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    };
                    oai_messages.push(OpenAiMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls,
                        tool_call_id: None,
                    });
                    continue;
                }

                for (tool_call_id, result_text) in tool_results {
                    oai_messages.push(OpenAiMessage {
                        role: "tool".into(),
                        content: Some(Value::String(result_text)),
                        tool_calls: None,
                        tool_call_id: Some(tool_call_id),
                    });
                }

                if !image_parts.is_empty() {
                    let mut content_parts = Vec::new();
                    if !text_parts.is_empty() {
                        content_parts.push(serde_json::json!({
                            "type": "text",
                            "text": text_parts.join("\n")
                        }));
                    }
                    content_parts.extend(image_parts);
                    oai_messages.push(OpenAiMessage {
                        role: "user".into(),
                        content: Some(Value::Array(content_parts)),
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

    Ok(oai_messages)
}

pub(crate) fn build_request(
    app: &AppHandle,
    messages: &[Message],
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<BuiltOpenAiRequest, ProviderTurnError> {
    let settings =
        crate::command::settings::get_settings(app.clone()).map_err(ProviderTurnError::new)?;
    let profile = settings.active_provider_profile();
    let builtin_tools = tools::get_available_tools();
    let tool_count = builtin_tools.len();

    let mut oai_messages = vec![OpenAiMessage {
        role: "system".into(),
        content: Some(Value::String(load_system_prompt(
            app,
            agent_mode,
            conversation_id,
        )?)),
        tool_calls: None,
        tool_call_id: None,
    }];
    oai_messages.extend(messages_to_openai_messages(messages)?);

    let tools = if builtin_tools.is_empty() {
        None
    } else {
        Some(
            builtin_tools
                .into_iter()
                .map(|tool| OpenAiTool {
                    r#type: "function".into(),
                    function: OpenAiFunction {
                        name: tool.name,
                        description: tool.description,
                        parameters: tool.input_schema,
                    },
                })
                .collect(),
        )
    };

    let request = OpenAiRequest {
        model: profile.model.clone(),
        messages: oai_messages,
        tools,
        stream_options: Some(OpenAiStreamOptions {
            include_usage: true,
        }),
        stream: true,
    };

    let input_tokens = compact::estimate_tokens_for_serializable(&request)
        .map(clamp_i64_to_u32)
        .map_err(ProviderTurnError::new)?;

    Ok(BuiltOpenAiRequest {
        request,
        estimate: ProviderPromptEstimate {
            input_tokens,
            source: "openai_chat_request",
            tool_count,
        },
    })
}
