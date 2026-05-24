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
