use tauri::AppHandle;

// Re-export all domain types and functions from the core workspace service
pub use crate::services::workspace::*;

#[tauri::command]
pub fn workspace_list_directory(
    app: AppHandle,
    conversation_id: Option<String>,
    path: Option<String>,
) -> Result<WorkspaceDirectoryListing, String> {
    let root = workspace_root_for_conversation(&app, conversation_id.as_deref())?;
    list_directory_for_root(root, path)
}

#[tauri::command]
pub fn workspace_read_text_file(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<WorkspaceFileContent, String> {
    let root = workspace_root_for_conversation(&app, conversation_id.as_deref())?;
    read_text_file(root, path)
}
