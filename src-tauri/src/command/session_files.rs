use tauri::AppHandle;

use crate::llm::services::session_files::{self, SessionFileMeta};
use crate::llm::utils::error_event::report_backend_result;

#[tauri::command]
pub async fn save_session_file(
    app: AppHandle,
    conversation_id: String,
    filename: String,
    content: Option<String>,
    raw_bytes: Option<Vec<u8>>,
) -> Result<SessionFileMeta, String> {
    let result = session_files::save_session_file(
        &app,
        &conversation_id,
        &filename,
        content.as_deref(),
        raw_bytes.as_deref(),
    );
    report_backend_result(&app, "command.session_files.save_session_file", result, None)
}

#[tauri::command]
pub async fn list_session_files(
    app: AppHandle,
    conversation_id: String,
) -> Result<Vec<SessionFileMeta>, String> {
    let result = session_files::list_session_files(&app, &conversation_id);
    report_backend_result(&app, "command.session_files.list_session_files", result, None)
}

#[tauri::command]
pub async fn read_session_file(
    app: AppHandle,
    read_path: String,
) -> Result<String, String> {
    let result = session_files::read_session_file(&read_path);
    report_backend_result(&app, "command.session_files.read_session_file", result, None)
}

pub async fn delete_all_session_files(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    session_files::delete_all_session_files(app, conversation_id)
}

pub async fn delete_all_session_files_all(app: &AppHandle) -> Result<(), String> {
    session_files::delete_all_session_files_all(app)
}
