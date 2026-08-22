use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 容忍显式 `null` 的反序列化助手：缺失或 null 都取类型默认值。
/// serde 的 `#[serde(default)]` 只处理"字段缺失"，而部分中转网关（OpenRouter
/// 的 Anthropic 兼容层）会把 cache_*_tokens 等计数字段显式发成 null，
/// 导致 u32 反序列化直接失败、整个回合被终止。
pub(crate) fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheControl {
    #[serde(rename = "type")]
    pub(crate) cache_type: String,
}
impl CacheControl {
    pub(crate) fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    pub(crate) block_type: &'static str,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<CacheControl>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicRequest {
    pub(crate) model: String,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) system: Option<Vec<AnthropicSystemBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) thinking: Option<AnthropicThinking>,
    pub(crate) messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) stop_sequences: Vec<String>,
    pub(crate) stream: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicResponse {
    pub(crate) content: Vec<AnthropicContentBlock>,
    pub(crate) usage: AnthropicUsage,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicThinking {
    #[serde(rename = "type")]
    pub(crate) thinking_type: String,
    pub(crate) budget_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: AnthropicMessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },

    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },

    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },

    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(default, skip_serializing_if = "is_false")]
        is_error: bool,
        content: Vec<AnthropicContentBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AnthropicImageSource {
    #[serde(rename = "type")]
    pub(crate) source_type: String,
    pub(crate) media_type: String,
    pub(crate) data: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<CacheControl>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct AnthropicUsage {
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) input_tokens: u32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) output_tokens: u32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) cache_read_input_tokens: u32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) cache_creation_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicResponse },

    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        #[serde(rename = "index")]
        _index: usize,
        content_block: StreamContentBlock,
    },

    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        #[serde(rename = "index")]
        _index: usize,
        delta: StreamDelta,
    },

    #[serde(rename = "content_block_stop")]
    ContentBlockStop {
        #[serde(rename = "index")]
        _index: usize,
    },

    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDelta,
        usage: StreamUsage,
    },

    #[serde(rename = "message_stop")]
    MessageStop,

    #[serde(rename = "ping")]
    Ping,

    #[serde(rename = "error")]
    Error { error: AnthropicStreamError },

    // 兜底：网关新增的未知事件类型收进这里容忍忽略，
    // 避免"unknown variant"直接终止整个回合（通知由适配器层发出）。
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicStreamError {
    #[serde(rename = "type")]
    pub(crate) error_type: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum StreamContentBlock {
    #[serde(rename = "text")]
    Text {
        #[serde(rename = "text")]
        _text: String,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(rename = "thinking")]
        _thinking: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        #[serde(rename = "input")]
        _input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum StreamDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageDelta {
    pub(crate) stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamUsage {
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) output_tokens: u32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) input_tokens: u32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) cache_read_input_tokens: u32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub(crate) cache_creation_input_tokens: u32,
}
