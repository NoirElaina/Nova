use serde::{Deserialize, Serialize};
use serde_json::Value;

// 发送给模型以及从模型接收的数据类型定义。
// 这些类型覆盖：
// 1) 通用对话消息结构
// 2) 工具调用与工具结果结构
// 3) Anthropic 请求/响应结构
// 4) 流式事件（SSE）解析结构

// 消息角色。
// 使用 lowercase 序列化，保证与上游 API 协议字段一致（"user" / "assistant"）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

// 对话执行模式：
// - Agent: 默认智能代理执行
// - Plan: 规划优先，不直接实现
// - Auto: 自动迭代，尽量一次完成闭环
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Agent,
    Plan,
    Auto,
}

// 消息内容：
// - Text: 纯文本
// - Blocks: 结构化块（文本、工具调用、工具结果）
// 这里使用 untagged 以兼容不同 provider 对 content 的不同 JSON 形态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

// 图片源（当前仅支持 base64）。
// 对应 Anthropic image block: source.type/media_type/data。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

// 结构化内容块。
// 通过 serde tag="type" 与外部协议对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    // 普通文本块。
    #[serde(rename = "text")]
    Text { text: String },

    // 推理/思考块。
    // signature 是 Anthropic 的密码学签名，回传时必须原样带回，否则 API 返回 400。
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },

    // 图片输入块（用于多模态请求）。
    #[serde(rename = "image")]
    Image { source: ImageSource },

    // 模型发起的工具调用请求。
    // id/name/input 对应 provider 的 tool_use 事件。
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    // 工具执行后的结果回填。
    // tool_use_id 对应 ToolUse 的 id，用于将结果与调用关联。
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        // 是否错误结果；兼容上游缺省字段。
        #[serde(default)]
        is_error: bool,
        // 工具结果内容也用块表示，便于支持文本/嵌套扩展。
        content: Vec<ContentBlock>,
    },
}

// 一条对话消息（role + content）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Content,
}

// 提供给模型的工具定义。
// input_schema 使用 JSON Schema（或兼容子集）描述参数结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// Anthropic 请求体（messages API）。
// stream=true 表示启用流式返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    // system 允许为空；为空时不序列化该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    // 若当前没有工具，省略 tools 字段，避免发送空数组造成兼容问题。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    pub stream: bool,
}

// Anthropic 非流式响应主体（或流式 message_start 中的 message）。
#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

// Token 使用量统计。
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// 流式事件类型（Anthropic SSE "data:" 事件）。
// 对应消息生命周期：start -> block_start/delta/stop -> message_delta -> message_stop。
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    // 一条新消息开始，包含 message 元数据。
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicResponse },

    // 一个内容块开始（可能是 text 或 tool_use）。
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: StreamContentBlock,
    },

    // 内容块增量（text_delta / input_json_delta）。
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: StreamDelta },

    // 内容块结束。
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },

    // 消息级增量（例如 stop_reason 更新、usage 更新）。
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDelta,
        usage: StreamUsage,
    },

    // 消息结束。
    #[serde(rename = "message_stop")]
    MessageStop,

    // 保活事件。
    #[serde(rename = "ping")]
    Ping,
}

// content_block_start 里可能出现的块类型。
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

// content_block_delta 的增量类型。
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamDelta {
    // 文本增量。
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    // 思考增量。
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    // Anthropic 思考签名增量，当前仅消费不展示。
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
    // 工具参数 JSON 片段增量。
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

// message_delta 的 delta 子结构。
#[derive(Debug, Deserialize)]
pub struct MessageDelta {
    pub stop_reason: Option<String>,
}

// 流式 usage 子结构（message_delta 中携带最终 token 统计）。
#[derive(Debug, Deserialize)]
pub struct StreamUsage {
    pub output_tokens: u32,
    // 部分 provider（如 mimo）在 message_delta 而非 message_start 里更新真实 input_tokens。
    #[serde(default)]
    pub input_tokens: u32,
}
