use serde::Serialize;
use tauri::AppHandle;

use crate::llm::types::{AgentMode, Message};

#[derive(Debug, Serialize, Clone)]
pub struct ChatMessageEvent {
    // 事件类型（text/tool-result/stop 等）。
    pub r#type: String,
    // 文本内容（按事件类型可选）。
    pub text: Option<String>,
    // 工具调用 ID（工具事件可选）。
    pub tool_use_id: Option<String>,
    // 工具名称（工具事件可选）。
    pub tool_use_name: Option<String>,
    // 工具输入（工具事件可选）。
    pub tool_use_input: Option<String>,
    // 工具输出（工具事件可选）。
    pub tool_result: Option<String>,
    // 工具结果是否为错误（tool-result 事件可选）。
    pub tool_is_error: Option<bool>,
    // token 使用量（可选）。
    pub token_usage: Option<u32>,
    // 停止原因（stop 事件可选）。
    pub stop_reason: Option<String>,
    // 回合状态（completed/error/cancelled 等）。
    pub turn_state: Option<String>,
    // 会话 ID（用于前端按会话分流流式事件）。
    pub conversation_id: Option<String>,
}

// 兼容入口：保留原函数名，内部委托给 query 模块实现。
pub async fn send_chat_message(
    app: AppHandle,
    conversation_id: Option<String>,
    messages: Vec<Message>,
    agent_mode: AgentMode,
) -> Result<(), String> {
    // 直接委托给新版 query 模块，保持旧 API 兼容。
    crate::llm::query::send_chat_message(app, conversation_id, messages, agent_mode).await
}
