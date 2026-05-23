use tauri::AppHandle;

pub use crate::llm::services::file_changes::FileChangeBatch;

#[tauri::command]
pub fn list_file_changes(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Vec<FileChangeBatch>, String> {
    crate::llm::services::file_changes::list_change_batches(&app, conversation_id.as_deref())
}

#[tauri::command]
pub fn revert_file_change(
    app: AppHandle,
    conversation_id: Option<String>,
    batch_id: String,
) -> Result<FileChangeBatch, String> {
    let root = crate::command::workspace::workspace_root_for_conversation(
        &app,
        conversation_id.as_deref(),
    )?;
    crate::llm::services::file_changes::revert_change_batch(
        &app,
        conversation_id.as_deref(),
        &root,
        &batch_id,
    )
}
