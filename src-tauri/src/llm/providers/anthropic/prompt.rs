use tauri::AppHandle;

use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError};
use crate::llm::services::compact;
use crate::llm::tools;
use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::model_context;
use crate::llm::utils::system_prompt::load_system_prompt;

use super::types::AnthropicRequest;

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

fn nova_messages_to_anthropic_messages(messages: &[Message]) -> Vec<Message> {
    messages.to_vec()
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
    let tools = tools::get_available_tools();
    let tool_count = tools.len();

    let request = AnthropicRequest {
        model: profile.model.clone(),
        max_tokens: model_context::get_max_output_tokens(&profile.model),
        system: Some(load_system_prompt(app, agent_mode, conversation_id)?),
        messages: nova_messages_to_anthropic_messages(messages),
        tools,
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
