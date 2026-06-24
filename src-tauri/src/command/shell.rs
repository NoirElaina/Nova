use crate::llm::services::shell_sessions::{self, ShellExecutionResult, ShellSessionStatus};
use crate::llm::utils::error_event::report_backend_result;
use tauri::AppHandle;

fn is_clear_shell_command(command: &str) -> bool {
    matches!(
        command.trim().to_ascii_lowercase().as_str(),
        "cls" | "clear" | "clear-host"
    )
}

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
    if is_clear_shell_command(command) {
        let cwd = shell_sessions::session_status(conversation_id.as_deref())
            .await
            .cwd
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| workspace_root.clone());
        return report_backend_result(
            &app,
            "command.shell.execute_shell_command",
            Ok(ShellExecutionResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
                cwd: Some(cwd),
                timed_out: false,
                cancelled: false,
                background: false,
                pid: None,
            }),
            Some("execute_shell_command_for_conversation"),
        );
    }

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
