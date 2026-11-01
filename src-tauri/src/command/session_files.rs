//! 桌面薄壳 IPC：转发至 nova daemon（方法名 session_files.* 与内核分发器一一对应）。

use nova_protocol::misc::*;



#[tauri::command]
pub async fn save_session_file(conversation_id: String, filename: String, content: Option<String>, raw_bytes: Option<Vec<u8>>) -> Result<SessionFileMeta, String> {
    crate::daemon::invoke("save_session_file", serde_json::json!({"conversationId": conversation_id, "filename": filename, "content": content, "rawBytes": raw_bytes})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_session_files(conversation_id: String) -> Result<Vec<SessionFileMeta>, String> {
    crate::daemon::invoke("list_session_files", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn read_session_file(conversation_id: String, filename: String) -> Result<String, String> {
    crate::daemon::invoke("read_session_file", serde_json::json!({"conversationId": conversation_id, "filename": filename})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
