//! 会话事件类型定义。
//!
//! 设计原则：
//! - 事件流是 append-only 的事实记录，模型可见内容必须能从中重建
//!   （Model-visible ⟺ Logged）；
//! - 消息类事件携带完整 `Message`（含 tool_use / thinking / image 块），
//!   UI 展示所需的文本/推理/附件由投影层提取，不提前拍平丢失结构。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::commands::types::HistoryAttachment;
use crate::llm::types::{Content, ContentBlock, Message, Role};

/// 会话事件：一行 `session_events` 的语义载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// 回合开始。
    TurnStart { turn_id: String },
    /// 回合结束（正常完成/取消/出错都算终态）。
    TurnEnd {
        turn_id: String,
        stop_reason: Option<String>,
    },

    /// 用户真实输入的消息（进模型上下文，也在 UI 聊天气泡展示）。
    UserMessage {
        message: Message,
        /// UI 附件元数据（模型内容块之外的展示信息）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<HistoryAttachment>>,
    },
    /// 助手输出（进模型上下文，也在 UI 聊天气泡展示；
    /// message 内含 thinking / tool_use 块，投影层负责提取文本与推理）。
    AssistantMessage {
        message: Message,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost: Option<Value>,
    },
    /// 其它进入模型上下文、但不在 UI 聊天气泡展示的消息
    /// （hook 注入上下文、工具结果回填消息、中断标记等）。
    ContextMessage { message: Message },

    /// 工具调用开始（与 ToolResult 按 call_id 配对投影出工具日志）。
    ToolCall {
        call_id: String,
        tool_name: String,
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
    },
    /// 工具调用结束。
    ToolResult {
        call_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
        started_at: i64,
        finished_at: i64,
    },

    /// 压缩检查点：payload 直接携带压缩完成后要发给模型的起点上下文。
    /// 重建规则：遇到该事件时丢弃其之前的全部事件，以 base_context 为新起点。
    CompactBoundary {
        base_context: Vec<Message>,
        summary: String,
        #[serde(default)]
        level: String,
        #[serde(default)]
        tokens_before: u32,
        #[serde(default)]
        tokens_after: u32,
    },

    /// 会话标题变化。
    TitleChanged { title: String },

    /// 发给模型 API 的 wire 级 HTTP 请求报文（含 system prompt/tools/消息数组完整结构）。
    WireRequest { url: String, body: Value },
    /// 模型 API 流结束后的完整响应 JSON（内容块/文本/stop_reason/用量）。
    WireResponse { body: Value },
}

impl SessionEvent {
    /// 事件类型名（入库的 event_type 列）。
    pub fn type_name(&self) -> &'static str {
        match self {
            SessionEvent::TurnStart { .. } => "turn_start",
            SessionEvent::TurnEnd { .. } => "turn_end",
            SessionEvent::UserMessage { .. } => "user_message",
            SessionEvent::AssistantMessage { .. } => "assistant_message",
            SessionEvent::ContextMessage { .. } => "context_message",
            SessionEvent::ToolCall { .. } => "tool_call",
            SessionEvent::ToolResult { .. } => "tool_result",
            SessionEvent::CompactBoundary { .. } => "compact_boundary",
            SessionEvent::TitleChanged { .. } => "title_changed",
            SessionEvent::WireRequest { .. } => "wire_request",
            SessionEvent::WireResponse { .. } => "wire_response",
        }
    }

    /// 把一条模型消息归类为对应事件：用户真实输入 / 助手输出。
    ///
    /// 例外：user 角色但内嵌 Image 块的消息是工具 side-channel 注入
    /// （ReadTool 读图 / ComputerUse 截图回灌，见 read.rs / computer_use.rs），
    /// 不是用户真实输入——落 ContextMessage，模型上下文保留、UI 不展示。
    /// 真实用户的图片经事件 attachments 字段携带，消息体内不出现 Image 块。
    pub fn from_model_message(message: Message) -> SessionEvent {
        match message.role {
            Role::User => {
                let has_inline_image = matches!(
                    &message.content,
                    Content::Blocks(blocks)
                        if blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. }))
                );
                if has_inline_image {
                    SessionEvent::ContextMessage { message }
                } else {
                    SessionEvent::UserMessage {
                        message,
                        attachments: None,
                    }
                }
            }
            Role::Assistant => SessionEvent::AssistantMessage {
                message,
                token_usage: None,
                cost: None,
            },
        }
    }
}
