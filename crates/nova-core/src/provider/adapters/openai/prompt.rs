use serde_json::Value;
use crate::core::Core;

use crate::provider::{ProviderPromptEstimate, ProviderTurnError};
use crate::agent::tools;
use crate::provider::types::{AgentMode, Content, ContentBlock, ImageSource, Message, Role};
use crate::agent::utils::system_prompt::load_system_prompt;
use crate::provider::token_counter;

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

struct ExtractedToolResult {
    tool_call_id: String,
    text: String,
    images: Vec<Value>,
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
                let mut tool_results: Vec<ExtractedToolResult> = Vec::new();

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
                            is_error,
                            content,
                        } => {
                            let mut texts = Vec::new();
                            let mut images = Vec::new();
                            for sub_block in content {
                                match sub_block {
                                    ContentBlock::Text { text } => {
                                        if !text.trim().is_empty() {
                                            texts.push(text.as_str());
                                        }
                                    }
                                    ContentBlock::Image { source } => {
                                        if let Some(part) = build_openai_image_part(source) {
                                            images.push(part);
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            let mut result_text = texts.join("\n");
                            if result_text.trim().is_empty() {
                                if !images.is_empty() {
                                    result_text = format!(
                                        "[Tool executed successfully with {} image output(s). Image content attached in follow-up message.]",
                                        images.len()
                                    );
                                } else if *is_error {
                                    result_text = "[Tool execution failed with empty output]".to_string();
                                } else {
                                    result_text = "[Tool execution completed with empty output]".to_string();
                                }
                            }

                            tool_results.push(ExtractedToolResult {
                                tool_call_id: tool_use_id.clone(),
                                text: result_text,
                                images,
                            });
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

                // 1. 先输出符合 OpenAI 规范的标准 tool 结果消息（严格保证 content 为 String）
                for res in &tool_results {
                    oai_messages.push(OpenAiMessage {
                        role: "tool".into(),
                        content: Some(Value::String(res.text.clone())),
                        tool_calls: None,
                        tool_call_id: Some(res.tool_call_id.clone()),
                    });
                }

                // 2. 紧随其后生成关联的多模态用户消息，供视觉模型消费工具生成的图像
                for res in &tool_results {
                    if !res.images.is_empty() {
                        let mut attached_parts = Vec::new();
                        attached_parts.push(serde_json::json!({
                            "type": "text",
                            "text": format!("Attached visual output from tool call '{}':", res.tool_call_id)
                        }));
                        attached_parts.extend(res.images.clone());
                        oai_messages.push(OpenAiMessage {
                            role: "user".into(),
                            content: Some(Value::Array(attached_parts)),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }

                // 3. 用户本身的普通文本与多模态图片输入
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
    core: &Core,
    messages: &[Message],
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<BuiltOpenAiRequest, ProviderTurnError> {
    let settings =
        crate::services::settings::load_settings(&core).map_err(ProviderTurnError::new)?;
    let profile = settings.active_provider_profile();
    let builtin_tools = tools::get_available_tools_for_agent(core, conversation_id);
    let tool_count = builtin_tools.len();

    let mut oai_messages = vec![OpenAiMessage {
        role: "system".into(),
        content: Some(Value::String(load_system_prompt(
            core,
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

    let input_tokens = token_counter::estimate_tokens_for_serializable(&request)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_result_with_image() {
        let tool_msg = Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_screenshot_1".to_string(),
                is_error: false,
                content: vec![
                    ContentBlock::Text {
                        text: "Captured webpage screenshot".to_string(),
                    },
                    ContentBlock::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: "image/png".to_string(),
                            data: "iVBORw0KGgoAAAANSUhEUgAA".to_string(),
                        },
                    },
                ],
            }]),
        };

        let result = messages_to_openai_messages(&[tool_msg]).unwrap();
        assert_eq!(result.len(), 2);

        // 验证第 1 条为合规的标准 tool 纯文本响应
        assert_eq!(result[0].role, "tool");
        assert_eq!(result[0].tool_call_id.as_deref(), Some("call_screenshot_1"));
        assert_eq!(
            result[0].content,
            Some(Value::String("Captured webpage screenshot".to_string()))
        );

        // 验证第 2 条为紧随其后的多模态附件消息
        assert_eq!(result[1].role, "user");
        let content_arr = result[1].content.as_ref().unwrap().as_array().unwrap();
        assert_eq!(content_arr.len(), 2);
        assert_eq!(content_arr[0]["type"], "text");
        assert_eq!(content_arr[1]["type"], "image_url");
        assert_eq!(
            content_arr[1]["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAA"
        );
    }

    #[test]
    fn test_tool_result_pure_image_generates_descriptive_placeholder() {
        let tool_msg = Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_pure_img".to_string(),
                is_error: false,
                content: vec![ContentBlock::Image {
                    source: ImageSource {
                        source_type: "base64".to_string(),
                        media_type: "image/jpeg".to_string(),
                        data: "abc123xyz".to_string(),
                    },
                }],
            }]),
        };

        let result = messages_to_openai_messages(&[tool_msg]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "tool");
        let tool_text = result[0].content.as_ref().unwrap().as_str().unwrap();
        assert!(tool_text.contains("1 image output"));

        assert_eq!(result[1].role, "user");
        let content_arr = result[1].content.as_ref().unwrap().as_array().unwrap();
        assert_eq!(content_arr[1]["image_url"]["url"], "data:image/jpeg;base64,abc123xyz");
    }
}
