pub mod adapters;
pub mod client;
pub mod sse_utils;
pub mod stream_runner;

use crate::llm::types::Message;

#[derive(Debug, Clone)]
pub struct ProviderTurnResult {
    pub messages: Vec<Message>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub prevent_continuation: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderPromptEstimate {
    pub input_tokens: u32,
    pub source: &'static str,
    pub tool_count: usize,
}

#[derive(Debug, Clone)]
pub struct ProviderTurnError {
    pub message: String,
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

impl From<String> for ProviderTurnError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

pub use client::LlmClient;
