use serde_json::Value;

use crate::provider::types::Message;

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

/// 挂钩生命周期事件：调用点只构造事件发射，统一由 dispatch::emit_hook_event 按事件名分发。
#[derive(Debug, Clone)]
pub enum HookEvent {
    SubagentStart { conversation_id: Option<String>, subagent_name: String },
    SubagentStop { conversation_id: Option<String>, subagent_name: String },
    PreToolUse { conversation_id: Option<String>, tool_name: String, input: Value },
    PostToolUse { conversation_id: Option<String>, tool_name: String, input: Value, output: String },
    PostToolUseFailure { conversation_id: Option<String>, tool_name: String, input: Value, error: String },
}
