use crate::llm::services::shell_sessions::{self, ShellSessionStatus};
use crate::llm::utils::error_event::report_backend_result;
use tauri::AppHandle;

#[tauri::command]
pub async fn get_shell_session_status(
    conversation_id: Option<String>,
) -> Result<ShellSessionStatus, String> {
    Ok(shell_sessions::session_status(conversation_id.as_deref()).await)
}

#[tauri::command]
pub async fn reset_shell_session_for_conversation(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.shell.reset_shell_session",
        shell_sessions::reset_session(conversation_id.as_deref()).await,
        Some("reset_shell_session"),
    )
}
