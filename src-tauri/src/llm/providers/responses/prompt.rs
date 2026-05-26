use serde::Serialize;
use tauri::AppHandle;

use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::services::compact;
use crate::llm::tools;
use crate::llm::types::{AgentMode, Content, ContentBlock, ImageSource, Message, Role};
use crate::llm::utils::model_context;
use crate::llm::utils::system_prompt::load_system_prompt;

use super::types::{
    ResponsesContentPart, ResponsesInputItem, ResponsesReasoningRequest,
    ResponsesReasoningSummaryPart, ResponsesRequest, ResponsesTextConfig, ResponsesTextFormat,
    ResponsesTool, ResponsesToolChoice, ResponsesTruncation,
};

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

#[derive(Serialize)]
#[serde(tag = "type")]
enum SerializedToolOutputBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        source_type: String,
        media_type: String,
        data: String,
    },
}

#[derive(Serialize)]
struct SerializedToolOutput {
    is_error: bool,
    content: Vec<SerializedToolOutputBlock>,
}

fn image_to_responses_part(source: &ImageSource) -> Option<ResponsesContentPart> {
    if !source.source_type.eq_ignore_ascii_case("base64") {
        return None;
    }

    let media_type = source.media_type.trim();
    let data = source.data.trim();
    if media_type.is_empty() || data.is_empty() {
        return None;
    }

    Some(ResponsesContentPart::InputImage {
        image_url: format!("data:{};base64,{}", media_type, data),
    })
}

fn push_message_if_any(
    input: &mut Vec<ResponsesInputItem>,
    role: &str,
    content: &mut Vec<ResponsesContentPart>,
) {
    if content.is_empty() {
        return;
    }

    input.push(ResponsesInputItem::Message {
        role: role.to_string(),
        content: std::mem::take(content),
    });
}

fn tool_result_output(
    content: &[ContentBlock],
    is_error: bool,
) -> Result<String, ProviderTurnError> {
    let mut text_blocks = Vec::new();
    let mut serialized_blocks = Vec::new();
    let mut has_non_text = false;

    for block in content {
        match block {
            ContentBlock::Text { text } => {
                text_blocks.push(text.clone());
                serialized_blocks.push(SerializedToolOutputBlock::Text { text: text.clone() });
            }
            ContentBlock::Image { source } => {
                has_non_text = true;
                serialized_blocks.push(SerializedToolOutputBlock::Image {
                    source_type: source.source_type.clone(),
                    media_type: source.media_type.clone(),
                    data: source.data.clone(),
                });
            }
            ContentBlock::Thinking { .. }
            | ContentBlock::ToolUse { .. }
            | ContentBlock::ToolResult { .. } => {
                return Err(ProviderTurnError::new(
                    "Responses function_call_output does not support nested thinking/tool blocks"
                        .to_string(),
                ));
            }
        }
    }

    if !has_non_text {
        return Ok(text_blocks.join("\n"));
    }

    serde_json::to_string(&SerializedToolOutput {
        is_error,
        content: serialized_blocks,
    })
    .map_err(|error| {
        ProviderTurnError::new(format!(
            "Failed to serialize Responses function_call_output content: {}",
            error
        ))
    })
}

fn messages_to_input(messages: &[Message]) -> Result<Vec<ResponsesInputItem>, ProviderTurnError> {
    let mut input = Vec::new();

    for message in messages {
        match message.role {
            Role::User => match &message.content {
                Content::Text(text) => {
                    input.push(ResponsesInputItem::Message {
                        role: "user".to_string(),
                        content: vec![ResponsesContentPart::InputText { text: text.clone() }],
                    });
                }
                Content::Blocks(blocks) => {
                    let mut content_parts = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                content_parts
                                    .push(ResponsesContentPart::InputText { text: text.clone() });
                            }
                            ContentBlock::Image { source } => {
                                if let Some(part) = image_to_responses_part(source) {
                                    content_parts.push(part);
                                }
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                is_error,
                                content,
                                ..
                            } => {
                                push_message_if_any(&mut input, "user", &mut content_parts);
                                input.push(ResponsesInputItem::FunctionCallOutput {
                                    call_id: tool_use_id.clone(),
                                    output: tool_result_output(content, *is_error)?,
                                });
                            }
                            ContentBlock::Thinking { .. } | ContentBlock::ToolUse { .. } => {
                                return Err(ProviderTurnError::new(
                                    "Responses user message cannot contain thinking or tool_use blocks"
                                        .to_string(),
                                ));
                            }
                        }
                    }

                    push_message_if_any(&mut input, "user", &mut content_parts);
                }
            },
            Role::Assistant => match &message.content {
                Content::Text(text) => {
                    input.push(ResponsesInputItem::Message {
                        role: "assistant".to_string(),
                        content: vec![ResponsesContentPart::OutputText { text: text.clone() }],
                    });
                }
                Content::Blocks(blocks) => {
                    let mut content_parts = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                content_parts
                                    .push(ResponsesContentPart::OutputText { text: text.clone() });
                            }
                            ContentBlock::Thinking { thinking, .. } => {
                                push_message_if_any(&mut input, "assistant", &mut content_parts);
                                input.push(ResponsesInputItem::Reasoning {
                                    summary: vec![ResponsesReasoningSummaryPart::SummaryText {
                                        text: thinking.clone(),
                                    }],
                                });
                            }
                            ContentBlock::ToolUse {
                                id,
                                name,
                                input: tool_input,
                            } => {
                                push_message_if_any(&mut input, "assistant", &mut content_parts);
                                let arguments =
                                    serde_json::to_string(tool_input).map_err(|error| {
                                        ProviderTurnError::new(format!(
                                            "Failed to serialize Responses API function call arguments for '{}': {}",
                                            name, error
                                        ))
                                    })?;
                                input.push(ResponsesInputItem::FunctionCall {
                                    call_id: id.clone(),
                                    name: name.clone(),
                                    arguments,
                                });
                            }
                            ContentBlock::Image { .. } | ContentBlock::ToolResult { .. } => {
                                return Err(ProviderTurnError::new(
                                    "Responses assistant message cannot contain image or tool_result blocks"
                                        .to_string(),
                                ));
                            }
                        }
                    }

                    push_message_if_any(&mut input, "assistant", &mut content_parts);
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
    let max_output_tokens = model_context::get_max_output_tokens(&profile.model);

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
        tool_choice: tools.as_ref().map(|_| ResponsesToolChoice::Auto),
        parallel_tool_calls: tools.as_ref().map(|_| true),
        tools,
        max_output_tokens,
        truncation: ResponsesTruncation::Disabled,
        reasoning: None::<ResponsesReasoningRequest>,
        text: ResponsesTextConfig {
            format: ResponsesTextFormat {
                format_type: "text".to_string(),
            },
        },
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
