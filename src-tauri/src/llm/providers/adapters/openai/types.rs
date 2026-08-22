use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream_options: Option<OpenAiStreamOptions>,
    pub(crate) stream: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiStreamOptions {
    pub(crate) include_usage: bool,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct OpenAiMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<OpenAiReqToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct OpenAiReqToolCall {
    pub(crate) id: String,
    pub(crate) r#type: String,
    pub(crate) function: OpenAiReqFunction,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct OpenAiReqFunction {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiTool {
    pub(crate) r#type: String,
    pub(crate) function: OpenAiFunction,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiFunction {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiStreamChunk {
    pub(crate) choices: Vec<OpenAiChoice>,
    #[serde(default)]
    pub(crate) usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) completion_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) total_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiPromptTokensDetails {
    #[serde(default)]
    pub(crate) cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiChoice {
    // 部分提供商的收尾 chunk 不带 delta，只给 finish_reason/usage，因此可选。
    #[serde(default)]
    pub(crate) delta: Option<OpenAiDelta>,
    // 非增量形态：部分 OpenRouter 上游/中转后端不流式推工具调用碎片，
    // 而是在单个 chunk 的 message 里一次性给出完整工具调用。
    #[serde(default)]
    pub(crate) message: Option<OpenAiStreamMessage>,
    pub(crate) finish_reason: Option<String>,
}

/// chunk 内完整形态的助手消息（非增量提供商使用）。
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiStreamMessage {
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<OpenAiCompleteToolCall>>,
}

/// 完整形态的工具调用：id/name/arguments 一次性给全。
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiCompleteToolCall {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<OpenAiFunctionCall>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiDelta {
    #[serde(default, rename = "role")]
    pub(crate) _role: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) refusal: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_content: Option<Value>,
    #[serde(default)]
    pub(crate) reasoning_details: Option<Value>,
    #[serde(default)]
    pub(crate) reasoning: Option<Value>,
    #[serde(default)]
    pub(crate) thinking_content: Option<Value>,
    pub(crate) tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiToolCall {
    // 少数提供商不带 index（通常一次只发一个完整工具调用），缺省回落到 0。
    #[serde(default)]
    pub(crate) index: Option<usize>,
    pub(crate) id: Option<String>,
    pub(crate) function: Option<OpenAiFunctionCall>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiFunctionCall {
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}
