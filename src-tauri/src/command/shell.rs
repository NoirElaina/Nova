use crate::llm::services::shell_sessions::{self, ShellExecutionResult, ShellSessionStatus};
use crate::llm::utils::error_event::report_backend_result;
use tauri::AppHandle;

#[tauri::command]
pub async fn get_shell_session_status(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<ShellSessionStatus, String> {
    let mut status = shell_sessions::session_status(conversation_id.as_deref()).await;
    if status.cwd.is_none() {
        status.cwd = Some(
            crate::command::workspace::workspace_root_string_for_conversation(
                &app,
                conversation_id.as_deref(),
            )?,
        );
    }
    Ok(status)
}

#[tauri::command]
pub async fn reset_shell_session_for_conversation(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<(), String> {
    let workspace_root = crate::command::workspace::workspace_root_string_for_conversation(
        &app,
        conversation_id.as_deref(),
    )?;
    report_backend_result(
        &app,
        "command.shell.reset_shell_session",
        shell_sessions::reset_session(conversation_id.as_deref(), Some(&workspace_root)).await,
        Some("reset_shell_session"),
    )
}

#[tauri::command]
pub async fn execute_shell_command_for_conversation(
    app: AppHandle,
    conversation_id: Option<String>,
    command: String,
    timeout_ms: Option<u64>,
    background: Option<bool>,
) -> Result<ShellExecutionResult, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Command cannot be empty".to_string());
    }

    let workspace_root = crate::command::workspace::workspace_root_string_for_conversation(
        &app,
        conversation_id.as_deref(),
    )?;
    let result = if background.unwrap_or(false) {
        shell_sessions::run_background(conversation_id.as_deref(), command, Some(&workspace_root))
            .await
    } else {
        shell_sessions::run_foreground(
            conversation_id.as_deref(),
            command,
            timeout_ms,
            Some(&workspace_root),
        )
        .await
    };

    report_backend_result(
        &app,
        "command.shell.execute_shell_command",
        result,
        Some("execute_shell_command_for_conversation"),
    )
}
