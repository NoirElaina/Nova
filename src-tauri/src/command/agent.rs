//! Agent 回合与分支问答 IPC（薄壳转发至 daemon）。

#[tauri::command]
pub async fn send_modern_agent_turn(
    conversation_id: String,
    prompt: String,
    attachments: Option<Vec<nova_protocol::session::HistoryAttachment>>,
) -> Result<(), String> {
    crate::daemon::invoke(
        "send_modern_agent_turn",
        serde_json::json!({
            "conversationId": conversation_id,
            "prompt": prompt,
            "attachments": attachments,
        }),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn cancel_modern_agent_turn(conversation_id: Option<String>) -> Result<bool, String> {
    crate::daemon::invoke(
        "cancel_modern_agent_turn",
        serde_json::json!({ "conversationId": conversation_id }),
    )
    .await
    .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn send_branch_message(
    parent_conversation_id: String,
    branch_id: String,
    messages: Vec<nova_protocol::llm::Message>,
) -> Result<(), String> {
    crate::daemon::invoke(
        "send_branch_message",
        serde_json::json!({
            "parentConversationId": parent_conversation_id,
            "branchId": branch_id,
            "messages": messages,
        }),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn cancel_branch_message(
    parent_conversation_id: String,
    branch_id: String,
) -> Result<bool, String> {
    crate::daemon::invoke(
        "cancel_branch_message",
        serde_json::json!({
            "parentConversationId": parent_conversation_id,
            "branchId": branch_id,
        }),
    )
    .await
    .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
