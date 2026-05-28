use tauri::AppHandle;

pub use crate::llm::services::file_changes::{FileChangeBatch, FileChangeBatchSummary};

#[tauri::command]
pub async fn list_file_changes(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Vec<FileChangeBatchSummary>, String> {
    crate::llm::services::file_changes::list_change_batches(&app, conversation_id.as_deref()).await
}

#[tauri::command]
pub async fn get_file_change(
    app: AppHandle,
    conversation_id: Option<String>,
    batch_id: String,
) -> Result<FileChangeBatch, String> {
    crate::llm::services::file_changes::get_change_batch(
        &app,
        conversation_id.as_deref(),
        &batch_id,
    )
    .await
}

#[tauri::command]
pub async fn revert_file_change(
    app: AppHandle,
    conversation_id: Option<String>,
    batch_id: String,
) -> Result<FileChangeBatch, String> {
    crate::llm::services::file_changes::revert_change_batch(
        &app,
        conversation_id.as_deref(),
        &batch_id,
    )
    .await
}
