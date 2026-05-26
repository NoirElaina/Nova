use tauri::AppHandle;

use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::services::compact;
use crate::llm::tools;
use crate::llm::types::{AgentMode, Content, ContentBlock, ImageSource, Message, Role, Tool};
use crate::llm::utils::model_context;
use crate::llm::utils::system_prompt::load_system_prompt;

use super::types::{
    AnthropicContentBlock, AnthropicImageSource, AnthropicMessage, AnthropicMessageContent,
    AnthropicRequest, AnthropicThinking, AnthropicTool,
};

pub(crate) struct BuiltAnthropicRequest {
    pub request: AnthropicRequest,
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

fn nova_role_to_anthropic_role(role: &Role) -> String {
    match role {
        Role::User => "user".to_string(),
        Role::Assistant => "assistant".to_string(),
    }
}

fn nova_image_source_to_anthropic(source: &ImageSource) -> AnthropicImageSource {
    AnthropicImageSource {
        source_type: source.source_type.clone(),
        media_type: source.media_type.clone(),
        data: source.data.clone(),
    }
}

fn nova_block_to_anthropic_block(
    block: &ContentBlock,
) -> Result<AnthropicContentBlock, ProviderTurnError> {
    match block {
        ContentBlock::Text { text } => Ok(AnthropicContentBlock::Text { text: text.clone() }),
        ContentBlock::Thinking {
            thinking,
            signature,
        } => Ok(AnthropicContentBlock::Thinking {
            thinking: thinking.clone(),
            signature: signature.clone(),
        }),
        ContentBlock::Image { source } => Ok(AnthropicContentBlock::Image {
            source: nova_image_source_to_anthropic(source),
        }),
        ContentBlock::ToolUse { id, name, input } => Ok(AnthropicContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        }),
        ContentBlock::ToolResult {
            tool_use_id,
            is_error,
            content,
        } => Ok(AnthropicContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            is_error: *is_error,
            content: nova_tool_result_content_to_anthropic(content)?,
        }),
    }
}

fn nova_tool_result_content_to_anthropic(
    blocks: &[ContentBlock],
) -> Result<Vec<AnthropicContentBlock>, ProviderTurnError> {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => Ok(AnthropicContentBlock::Text { text: text.clone() }),
            ContentBlock::Image { source } => Ok(AnthropicContentBlock::Image {
                source: nova_image_source_to_anthropic(source),
            }),
            ContentBlock::Thinking { .. }
            | ContentBlock::ToolUse { .. }
            | ContentBlock::ToolResult { .. } => Err(ProviderTurnError::new(
                "Anthropic tool_result content only supports text and image blocks".to_string(),
            )),
        })
        .collect()
}

fn nova_content_to_anthropic_content(
    content: &Content,
) -> Result<AnthropicMessageContent, ProviderTurnError> {
    match content {
        Content::Text(text) => Ok(AnthropicMessageContent::Text(text.clone())),
        Content::Blocks(blocks) => blocks
            .iter()
            .map(nova_block_to_anthropic_block)
            .collect::<Result<Vec<_>, _>>()
            .map(AnthropicMessageContent::Blocks),
    }
}

fn nova_messages_to_anthropic_messages(
    messages: &[Message],
) -> Result<Vec<AnthropicMessage>, ProviderTurnError> {
    messages
        .iter()
        .map(|message| {
            Ok(AnthropicMessage {
                role: nova_role_to_anthropic_role(&message.role),
                content: nova_content_to_anthropic_content(&message.content)?,
            })
        })
        .collect()
}

fn nova_tools_to_anthropic_tools(tools: Vec<Tool>) -> Vec<AnthropicTool> {
    tools
        .into_iter()
        .map(|tool| AnthropicTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
        })
        .collect()
}

fn anthropic_stop_sequences(profile: &crate::command::settings::ProviderProfile) -> Vec<String> {
    profile.stop_sequences.clone()
}

fn anthropic_thinking(
    profile: &crate::command::settings::ProviderProfile,
    max_tokens: u32,
) -> Result<Option<AnthropicThinking>, ProviderTurnError> {
    if !profile.anthropic_thinking_enabled {
        return Ok(None);
    }

    let budget_tokens = profile.anthropic_thinking_budget_tokens.unwrap_or(1024);
    if budget_tokens < 1024 {
        return Err(ProviderTurnError::new(
            "Anthropic thinking budget must be at least 1024 tokens".to_string(),
        ));
    }
    if budget_tokens >= max_tokens {
        return Err(ProviderTurnError::new(format!(
            "Anthropic thinking budget ({}) must be less than max_tokens ({})",
            budget_tokens, max_tokens
        )));
    }

    Ok(Some(AnthropicThinking {
        thinking_type: "enabled".to_string(),
        budget_tokens,
    }))
}

pub(crate) fn build_request(
    app: &AppHandle,
    messages: &[Message],
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<BuiltAnthropicRequest, ProviderTurnError> {
    let settings =
        crate::command::settings::get_settings(app.clone()).map_err(ProviderTurnError::new)?;
    let profile = settings.active_provider_profile();
    let nova_tools = tools::get_available_tools();
    let tool_count = nova_tools.len();
    let max_tokens = model_context::get_max_output_tokens(&profile.model);

    let request = AnthropicRequest {
        model: profile.model.clone(),
        max_tokens,
        system: Some(load_system_prompt(app, agent_mode, conversation_id)?),
        thinking: anthropic_thinking(&profile, max_tokens)?,
        messages: nova_messages_to_anthropic_messages(messages)?,
        tools: nova_tools_to_anthropic_tools(nova_tools),
        stop_sequences: anthropic_stop_sequences(&profile),
        stream: true,
    };

    let input_tokens = compact::estimate_tokens_for_serializable(&request)
        .map(clamp_i64_to_u32)
        .map_err(ProviderTurnError::new)?;

    Ok(BuiltAnthropicRequest {
        request,
        estimate: ProviderPromptEstimate {
            input_tokens,
            source: "anthropic_request",
            tool_count,
        },
    })
}
