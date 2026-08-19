use tauri::AppHandle;

use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::tools;
use crate::llm::types::{AgentMode, Content, ContentBlock, ImageSource, Message, Role, Tool};
use crate::llm::utils::model_context;
use crate::llm::utils::system_prompt::load_system_prompt;
use crate::llm::utils::token_counter;

use super::types::{
    AnthropicContentBlock, AnthropicImageSource, AnthropicMessage, AnthropicMessageContent,
    AnthropicRequest, AnthropicSystemBlock, AnthropicThinking, AnthropicTool, CacheControl,
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
        ContentBlock::Text { text } => Ok(AnthropicContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        }),
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
            cache_control: None,
        }),
    }
}

fn nova_tool_result_content_to_anthropic(
    blocks: &[ContentBlock],
) -> Result<Vec<AnthropicContentBlock>, ProviderTurnError> {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => Ok(AnthropicContentBlock::Text {
                text: text.clone(),
                cache_control: None,
            }),
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
    let count = tools.len();
    tools
        .into_iter()
        .enumerate()
        .map(|(index, tool)| AnthropicTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
            // 最后一个工具挂 ephemeral：把 system + 完整工具定义纳入缓存前缀。
            // 工具描述很长（几 K token），不缓存则每轮全价重算。
            cache_control: if index + 1 == count {
                Some(CacheControl::ephemeral())
            } else {
                None
            },
        })
        .collect()
}

/// 消息历史增量缓存断点：给最后一条消息的最后一个可标记块（Text/ToolResult）挂 ephemeral。
/// 每轮请求重建消息数组，断点总是落在当前最后一条消息上——
/// 上一轮创建的缓存前缀（system + tools + 旧消息）本轮命中，仅新增部分走 cache_creation。
/// 与 system、tools 两处断点合计 3 个，低于 Anthropic 的 4 个上限。
fn mark_last_message_cacheable(messages: &mut [AnthropicMessage]) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    match &mut last.content {
        AnthropicMessageContent::Text(text) => {
            let text = std::mem::take(text);
            last.content = AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::Text {
                text,
                cache_control: Some(CacheControl::ephemeral()),
            }]);
        }
        AnthropicMessageContent::Blocks(blocks) => {
            for block in blocks.iter_mut().rev() {
                match block {
                    AnthropicContentBlock::Text { cache_control, .. }
                    | AnthropicContentBlock::ToolResult { cache_control, .. } => {
                        *cache_control = Some(CacheControl::ephemeral());
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn anthropic_stop_sequences(profile: &crate::command::settings::ProviderProfile) -> Vec<String> {
    profile.stop_sequences.clone()
}

/// thinking 预算解析。
/// - 未配置（None）→ 视为"不限"：取 API 允许的最大值 max_tokens-1，模型想多长想多长。
///   budget 只是上限不是目标，模型思考完会自行停止，不会每次都花满。
/// - 显式配置了值 → 仍尊重用户设置，但夹到 [1024, max_tokens-1]，
///   超出上限不再报错（旧行为是整个请求失败）。
/// - max_tokens 本身过小（<= 1024）时无法满足 API 下限，静默关闭 thinking。
fn anthropic_thinking(
    profile: &crate::command::settings::ProviderProfile,
    max_tokens: u32,
) -> Result<Option<AnthropicThinking>, ProviderTurnError> {
    if !profile.anthropic_thinking_enabled {
        return Ok(None);
    }

    // Anthropic 硬性要求：1024 <= budget_tokens < max_tokens。
    let hard_max = max_tokens.saturating_sub(1);
    if hard_max < 1024 {
        return Ok(None);
    }

    let budget_tokens = match profile.anthropic_thinking_budget_tokens {
        None => hard_max,
        Some(budget) => budget.clamp(1024, hard_max),
    };

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
    let nova_tools = tools::get_available_tools_for_agent(app, conversation_id);
    let tool_count = nova_tools.len();
    let max_tokens = model_context::get_max_output_tokens(&profile.model);

    let mut messages = nova_messages_to_anthropic_messages(messages)?;
    mark_last_message_cacheable(&mut messages);

    let request = AnthropicRequest {
        model: profile.model.clone(),
        max_tokens,
        system: Some(vec![AnthropicSystemBlock {
            block_type: "text",
            text: load_system_prompt(app, agent_mode, conversation_id)?,
            cache_control: Some(CacheControl::ephemeral()),
        }]),
        thinking: anthropic_thinking(&profile, max_tokens)?,
        messages,
        tools: nova_tools_to_anthropic_tools(nova_tools),
        stop_sequences: anthropic_stop_sequences(&profile),
        stream: true,
    };

    let input_tokens = token_counter::estimate_tokens_for_serializable(&request)
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
