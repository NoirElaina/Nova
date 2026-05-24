use crate::llm::services::user_terminal::{self, UserTerminalInfo};
use tauri::AppHandle;

#[tauri::command]
pub fn user_terminal_start(
    app: AppHandle,
    conversation_id: Option<String>,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<UserTerminalInfo, String> {
    let workspace_root = crate::command::workspace::workspace_root_for_conversation(
        &app,
        conversation_id.as_deref(),
    )?;
    user_terminal::start_session(app, conversation_id.as_deref(), &workspace_root, rows, cols)
}

#[tauri::command]
pub fn user_terminal_write(conversation_id: Option<String>, data: String) -> Result<(), String> {
    user_terminal::write_session(conversation_id.as_deref(), data)
}

#[tauri::command]
pub fn user_terminal_resize(
    conversation_id: Option<String>,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<(), String> {
    user_terminal::resize_session(conversation_id.as_deref(), rows, cols)
}

#[tauri::command]
pub fn user_terminal_stop(conversation_id: Option<String>) -> Result<(), String> {
    user_terminal::stop_session(conversation_id.as_deref())
}
