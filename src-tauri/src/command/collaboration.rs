//! 桌面薄壳 IPC：协同看板与多智能体拓扑转发至 nova daemon。

use serde_json::Value;

#[tauri::command]
pub async fn get_agent_graph(conversation_id: String) -> Result<Value, String> {
    crate::daemon::invoke("get_agent_graph", serde_json::json!({ "conversationId": conversation_id })).await
}

#[tauri::command]
pub async fn get_board_channels(conversation_id: String) -> Result<Value, String> {
    crate::daemon::invoke("get_board_channels", serde_json::json!({ "conversationId": conversation_id })).await
}

#[tauri::command]
pub async fn get_board_posts(
    conversation_id: String,
    channel: Option<String>,
    thread_id: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    crate::daemon::invoke("get_board_posts", serde_json::json!({
        "conversationId": conversation_id,
        "channel": channel,
        "threadId": thread_id,
        "limit": limit
    })).await
}

#[tauri::command]
pub async fn post_board_message(
    conversation_id: String,
    channel: String,
    content: String,
    thread_id: Option<String>,
    author_nickname: Option<String>,
    author_role: Option<String>,
    idempotency_key: Option<String>,
) -> Result<Value, String> {
    crate::daemon::invoke("post_board_message", serde_json::json!({
        "conversationId": conversation_id,
        "channel": channel,
        "content": content,
        "threadId": thread_id,
        "authorNickname": author_nickname,
        "authorRole": author_role,
        "idempotencyKey": idempotency_key
    })).await
}

#[tauri::command]
pub async fn interrupt_agent(conversation_id: String, agent_id: String) -> Result<bool, String> {
    crate::daemon::invoke("interrupt_agent", serde_json::json!({
        "conversationId": conversation_id,
        "agentId": agent_id
    })).await
    .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn send_agent_message(
    conversation_id: String,
    agent_id: String,
    directive: String,
) -> Result<bool, String> {
    crate::daemon::invoke("send_agent_message", serde_json::json!({
        "conversationId": conversation_id,
        "agentId": agent_id,
        "directive": directive
    })).await
    .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
