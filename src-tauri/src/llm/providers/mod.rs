// Anthropic Messages API provider adapter.
pub mod anthropic;
// OpenAI Chat Completions provider adapter.
pub mod openai;
// OpenAI Responses API provider adapter.
pub mod responses;
// 共享 SSE 解析工具函数。
pub mod sse_utils;
// 共享流式运行器（StreamParser trait + run_streaming + Delta）。
pub mod stream_runner;

use crate::llm::types::{AgentMode, Message};
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct ProviderTurnResult {
    // 本轮生成并需要并入上下文的消息。
    pub messages: Vec<Message>,
    // 本轮停止原因（可选）。
    pub stop_reason: Option<String>,
    // provider 上报的输入 token 数（可选）。
    pub input_tokens: Option<u32>,
    // provider 上报的输出 token 数（可选）。
    pub output_tokens: Option<u32>,
    // 是否阻止 query 层继续发起下一轮。
    pub prevent_continuation: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderPromptEstimate {
    pub input_tokens: u32,
    pub source: &'static str,
    pub tool_count: usize,
}

/// 流式请求失败时携带的错误信息。
/// 除错误文本外，还包含流中断前已生成的 partial 消息，
/// 供上层保存 snapshot 避免上下文丢失。
#[derive(Debug, Clone)]
pub struct ProviderTurnError {
    pub message: String,
    /// 流中断前已生成的 partial assistant 消息（可能为空）。
    pub partial_messages: Vec<Message>,
}

impl ProviderTurnError {
    pub fn new(message: String) -> Self {
        Self {
            message,
            partial_messages: Vec::new(),
        }
    }
    pub fn with_partial(message: String, partial_messages: Vec<Message>) -> Self {
        Self {
            message,
            partial_messages,
        }
    }
}

/// 允许 `?` 运算符自动从 `String` 转换（用于 load_system_prompt 等场合）。
impl From<String> for ProviderTurnError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

pub enum LlmProvider {
    // Anthropic provider 分支。
    Anthropic(anthropic::AnthropicProvider),
    // OpenAI Chat Completions provider 分支。
    OpenAi(openai::OpenAiProvider),
    // OpenAI Responses API provider 分支。
    Responses(responses::ResponsesProvider),
}

impl LlmProvider {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        // 读取运行时设置。
        let settings = crate::command::settings::get_settings(app.clone())?;
        // profile key 只负责选中配置；真正路由按 profile.protocol 判断。
        let protocol = settings.active_provider_protocol();

        // Anthropic 协议走 AnthropicProvider，openai_responses 走 ResponsesProvider，其余默认走 OpenAI 兼容协议实现。
        if protocol == "anthropic" {
            Ok(LlmProvider::Anthropic(anthropic::AnthropicProvider))
        } else if protocol == "openai_responses" {
            Ok(LlmProvider::Responses(responses::ResponsesProvider))
        } else {
            Ok(LlmProvider::OpenAi(openai::OpenAiProvider))
        }
    }

    pub async fn send_request(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, ProviderTurnError> {
        // 根据当前枚举分支转发到具体 provider 实现。
        match self {
            LlmProvider::Anthropic(p) => {
                p.send_request(app, messages, agent_mode, conversation_id)
                    .await
            }
            LlmProvider::OpenAi(p) => {
                p.send_request(app, messages, agent_mode, conversation_id)
                    .await
            }
            LlmProvider::Responses(p) => {
                p.send_request(app, messages, agent_mode, conversation_id)
                    .await
            }
        }
    }

    pub fn estimate_prompt_tokens(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderPromptEstimate, ProviderTurnError> {
        match self {
            LlmProvider::Anthropic(_) => {
                anthropic::prompt::build_request(app, messages, agent_mode, conversation_id)
                    .map(|built| built.estimate)
            }
            LlmProvider::OpenAi(_) => {
                openai::prompt::build_request(app, messages, agent_mode, conversation_id)
                    .map(|built| built.estimate)
            }
            LlmProvider::Responses(_) => {
                responses::prompt::build_request(app, messages, agent_mode, conversation_id)
                    .map(|built| built.estimate)
            }
        }
    }
}
