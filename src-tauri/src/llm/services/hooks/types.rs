use serde_json::Value;

use crate::llm::types::Message;

#[derive(Debug, Default, Clone)]
pub struct HookOutcome {
    pub additional_messages: Vec<Message>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
    pub override_error: Option<String>,
}

impl HookOutcome {
    pub fn from_error(error: String) -> Self {
        Self {
            override_error: Some(error),
            prevent_continuation: true,
            ..Self::default()
        }
    }
}

/// 挂钩生命周期事件：调用点只构造事件发射，不再直接调具体 run_* 函数。
/// 统一由 dispatch::emit_hook_event 按事件名查 hooks.toml 分发。
#[derive(Debug, Clone)]
pub enum HookEvent {
    SessionStart { conversation_id: Option<String> },
    UserPromptSubmit { conversation_id: Option<String> },
    PreCompact { conversation_id: Option<String> },
    PostCompact { conversation_id: Option<String> },
    SessionEnd { conversation_id: Option<String>, stop_reason: String },
    Error { conversation_id: Option<String>, error: String },
    SubagentStart { conversation_id: Option<String>, subagent_name: String },
    SubagentStop { conversation_id: Option<String>, subagent_name: String },
    PreToolUse { conversation_id: Option<String>, tool_name: String, input: Value },
    PostToolUse { conversation_id: Option<String>, tool_name: String, input: Value, output: String },
    PostToolUseFailure { conversation_id: Option<String>, tool_name: String, input: Value, error: String },
    /// Stop 事件携带当前消息历史（助手消息计数与上下文去重用）。
    Stop { conversation_id: Option<String>, messages: Vec<Message> },
}
