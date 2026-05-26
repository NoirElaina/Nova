use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::services::compact;
use crate::llm::tools;
use crate::llm::types::{AgentMode, Content, ContentBlock, Message, Role};
use crate::llm::utils::system_prompt::load_system_prompt;

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequest {
    model: String,
    input: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool>>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ResponsesTool {
    r#type: String,
    name: String,
    description: String,
    parameters: Value,
}

pub(crate) struct BuiltResponsesRequest {
    pub request: ResponsesRequest,
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

fn messages_to_input(messages: &[Message]) -> Result<Vec<Value>, ProviderTurnError> {
    let mut input = Vec::new();

    for message in messages {
        match message.role {
            Role::User => match &message.content {
                Content::Text(text) => {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": text }]
                    }));
                }
                Content::Blocks(blocks) => {
                    let mut content_parts = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                content_parts.push(serde_json::json!({
                                    "type": "input_text",
                                    "text": text
                                }));
                            }
                            ContentBlock::Image { source } => {
                                if source.source_type.eq_ignore_ascii_case("base64")
                                    && !source.media_type.is_empty()
                                    && !source.data.is_empty()
                                {
                                    content_parts.push(serde_json::json!({
                                        "type": "input_image",
                                        "image_url": format!("data:{};base64,{}", source.media_type, source.data)
                                    }));
                                }
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
                                input.push(serde_json::json!({
                                    "type": "function_call_output",
                                    "call_id": tool_use_id,
                                    "output": text
                                }));
                            }
                            _ => {}
                        }
                    }

                    if !content_parts.is_empty() {
                        input.push(serde_json::json!({
                            "type": "message",
                            "role": "user",
                            "content": content_parts
                        }));
                    }
                }
            },
            Role::Assistant => match &message.content {
                Content::Text(text) => {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }]
                    }));
                }
                Content::Blocks(blocks) => {
                    let mut text_content = Vec::new();
                    let mut tool_uses = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                text_content.push(text.as_str());
                            }
                            ContentBlock::ToolUse {
                                id,
                                name,
                                input: tool_input,
                            } => {
                                tool_uses.push((id, name, tool_input));
                            }
                            _ => {}
                        }
                    }

                    if !text_content.is_empty() {
                        input.push(serde_json::json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text_content.join("\n") }]
                        }));
                    }

                    for (id, name, tool_input) in tool_uses {
                        let arguments = serde_json::to_string(tool_input).map_err(|error| {
                            ProviderTurnError::new(format!(
                                "Failed to serialize Responses API function call arguments for '{}': {}",
                                name, error
                            ))
                        })?;
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": arguments
                        }));
                    }
                }
            },
        }
    }

    Ok(input)
}

pub(crate) fn build_request(
    app: &AppHandle,
    messages: &[Message],
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<BuiltResponsesRequest, ProviderTurnError> {
    let settings =
        crate::command::settings::get_settings(app.clone()).map_err(ProviderTurnError::new)?;
    let profile = settings.active_provider_profile();
    let builtin_tools = tools::get_available_tools();
    let tool_count = builtin_tools.len();

    let tools = if builtin_tools.is_empty() {
        None
    } else {
        Some(
            builtin_tools
                .into_iter()
                .map(|tool| ResponsesTool {
                    r#type: "function".into(),
                    name: tool.name,
                    description: tool.description,
                    parameters: tool.input_schema,
                })
                .collect(),
        )
    };

    let request = ResponsesRequest {
        model: profile.model.clone(),
        input: messages_to_input(messages)?,
        instructions: Some(load_system_prompt(app, agent_mode, conversation_id)?),
        tools,
        stream: true,
    };

    let input_tokens = compact::estimate_tokens_for_serializable(&request)
        .map(clamp_i64_to_u32)
        .map_err(ProviderTurnError::new)?;

    Ok(BuiltResponsesRequest {
        request,
        estimate: ProviderPromptEstimate {
            input_tokens,
            source: "openai_responses_request",
            tool_count,
        },
    })
}
