//! 桌面薄壳 IPC：转发至 nova daemon（方法名 history.* 与内核分发器一一对应）。

pub use nova_protocol::session::{
    ConversationMeta, HistoryMessage, HistoryToolExecution,
};

#[tauri::command]
pub async fn create_conversation(title: Option<String>, workspace_path: Option<String>) -> Result<ConversationMeta, String> {
    crate::daemon::invoke("create_conversation", serde_json::json!({"title": title, "workspacePath": workspace_path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_conversations() -> Result<Vec<ConversationMeta>, String> {
    crate::daemon::invoke("list_conversations", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn set_conversation_pinned(conversation_id: String, pinned: bool) -> Result<(), String> {
    crate::daemon::invoke("set_conversation_pinned", serde_json::json!({"conversationId": conversation_id, "pinned": pinned})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn export_conversation(conversation_id: String, format: String) -> Result<String, String> {
    crate::daemon::invoke("export_conversation", serde_json::json!({"conversationId": conversation_id, "format": format})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn export_rendered_conversation_pdf(conversation_id: String, title: String, html: String) -> Result<String, String> {
    crate::daemon::invoke("export_rendered_conversation_pdf", serde_json::json!({"conversationId": conversation_id, "title": title, "html": html})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn load_history(conversation_id: String) -> Result<Vec<HistoryMessage>, String> {
    crate::daemon::invoke("load_history", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn replace_history(conversation_id: String, messages: Vec<HistoryMessage>) -> Result<(), String> {
    crate::daemon::invoke("replace_history", serde_json::json!({"conversationId": conversation_id, "messages": messages})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn append_plain_chat_message(conversation_id: String, role: String, content: String) -> Result<(), String> {
    crate::daemon::invoke("append_plain_chat_message", serde_json::json!({"conversationId": conversation_id, "role": role, "content": content})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn load_conversation_tool_logs(conversation_id: String) -> Result<Vec<HistoryToolExecution>, String> {
    crate::daemon::invoke("load_conversation_tool_logs", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_conversation_turn_traces(conversation_id: String) -> Result<Vec<nova_protocol::misc::TurnTrace>, String> {
    crate::daemon::invoke("get_conversation_turn_traces", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn clear_history(conversation_id: Option<String>) -> Result<(), String> {
    crate::daemon::invoke("clear_history", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn delete_conversation(conversation_id: String) -> Result<(), String> {
    crate::daemon::invoke("delete_conversation", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_memory_entries() -> Result<Vec<String>, String> {
    crate::daemon::invoke("list_memory_entries", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn add_memory_entry(content: String) -> Result<(), String> {
    crate::daemon::invoke("add_memory_entry", serde_json::json!({"content": content})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn remove_memory_entry(old_text: String) -> Result<(), String> {
    crate::daemon::invoke("remove_memory_entry", serde_json::json!({"oldText": old_text})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn clear_memory_entries() -> Result<(), String> {
    crate::daemon::invoke("clear_memory_entries", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn manual_compact_conversation(conversation_id: String) -> Result<nova_protocol::misc::ManualCompactOutcome, String> {
    crate::daemon::invoke("manual_compact_conversation", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
