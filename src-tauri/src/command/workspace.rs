//! 桌面薄壳 IPC：转发至 nova daemon（方法名 workspace.* 与内核分发器一一对应）。

use nova_protocol::misc::*;



#[tauri::command]
pub async fn workspace_list_directory(conversation_id: Option<String>, path: Option<String>) -> Result<WorkspaceDirectoryListing, String> {
    crate::daemon::invoke("workspace_list_directory", serde_json::json!({"conversationId": conversation_id, "path": path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn workspace_read_text_file(conversation_id: Option<String>, path: String) -> Result<WorkspaceFileContent, String> {
    crate::daemon::invoke("workspace_read_text_file", serde_json::json!({"conversationId": conversation_id, "path": path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
