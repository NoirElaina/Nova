use serde::{Deserialize, Serialize};
use serde_json::Value;

// Nova 内部对话消息结构。
// Provider adapter 会在发送前把这些语义块转换成各自协议格式。

// Nova 内部消息角色。
// lowercase 序列化用于持久化与 provider adapter 输入，不代表绑定某个上游协议。
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

// Nova 结构化内容块。
// 这里的 tag 是 Nova 内部持久化标签；各 provider adapter 负责翻译成上游协议字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    // 普通文本块。
    #[serde(rename = "text")]
    Text { text: String },

    // 推理/思考块。signature 是 provider 可能要求回传的不透明签名。
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },

    // 图片输入块（用于多模态请求）。
    #[serde(rename = "image")]
    Image { source: ImageSource },

    // 模型发起的工具调用请求。
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

// Nova 暴露给模型的工具定义。
// input_schema 使用 JSON Schema（或兼容子集）描述参数结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
