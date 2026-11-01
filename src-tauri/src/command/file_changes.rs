//! 桌面薄壳 IPC：转发至 nova daemon（方法名 file_changes.* 与内核分发器一一对应）。

use nova_protocol::misc::*;

#[tauri::command]
pub async fn get_workspace_diff(conversation_id: Option<String>) -> Result<WorkspaceDiff, String> {
    crate::daemon::invoke("get_workspace_diff", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_git_repo_status(conversation_id: Option<String>) -> Result<GitRepoStatus, String> {
    crate::daemon::invoke("get_git_repo_status", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_workspace_git_status(workspace_path: String) -> Result<GitRepoStatus, String> {
    crate::daemon::invoke("get_workspace_git_status", serde_json::json!({"workspacePath": workspace_path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn init_git_repo(conversation_id: Option<String>) -> Result<InitGitRepoResult, String> {
    crate::daemon::invoke("init_git_repo", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
