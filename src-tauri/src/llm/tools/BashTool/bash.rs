use crate::llm::services::shell_sessions::ShellExecutionResult;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission))
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    crate::llm::utils::permissions::describe_shell_command_permission(
        "Bash",
        "终端命令",
        input,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "Bash".into(),
        description: r#"Execute a bash/zsh/pwsh command in a conversation-scoped persistent shell session. The session keeps its working directory and environment between calls.

- `command`: the command to execute (required).
- `description`: a short (3-5 word) description of what this command does in active voice. Helps the user understand what's happening.
- `timeout`: optional timeout in milliseconds (max 600000). Defaults to 120000.
- `run_in_background`: set to true to run the command in the background. The shell session stays alive for subsequent calls.

On Windows this runs PowerShell 7 (pwsh). On Linux/macOS it runs sh. Interactive TUI programs are not supported."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does in active voice"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds (max 600000)"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background."
                }
            },
            "required": ["command"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let cmd = match input.get("command").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.to_string(),
        _ => return Err(ToolFailure::invalid_input("Missing 'command' argument")),
    };
    let timeout_ms = input
        .get("timeout")
        .and_then(|value| value.as_u64());
    let background = input
        .get("run_in_background")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let workspace_root =
        match crate::command::workspace::workspace_root_string_for_conversation(app, conversation_id)
        {
            Ok(root) => root,
            Err(error) => {
                return Err(ToolFailure::new(format!(
                    "Failed to resolve workspace: {}",
                    error
                )));
            }
        };

    let result = if background {
        crate::llm::services::shell_sessions::run_background(
            conversation_id,
            &cmd,
            Some(&workspace_root),
        )
        .await
    } else {
        crate::llm::services::shell_sessions::run_foreground(
            conversation_id,
            &cmd,
            timeout_ms,
            Some(&workspace_root),
        )
        .await
    };

    match result {
        Ok(result) if result.cancelled => Err(ToolFailure::cancelled(shell_failure_text(
            "command cancelled",
            &result,
        ))),
        Ok(result) if result.timed_out => Err(ToolFailure::new(shell_failure_text(
            "command timed out",
            &result,
        ))),
        Ok(result) => Ok(ToolOutcome::json(shell_result_json(result))),
        Err(error) => Err(ToolFailure::new(format!(
            "Failed to execute command: {}",
            error
        ))),
    }
}

fn shell_result_json(result: ShellExecutionResult) -> Value {
    json!({
        "ok": result.exit_code.unwrap_or(1) == 0,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exitCode": result.exit_code,
        "cwd": result.cwd,
        "timedOut": result.timed_out,
        "background": result.background,
        "pid": result.pid
    })
}

fn shell_failure_text(reason: &str, result: &ShellExecutionResult) -> String {
    format!(
        "{reason}\nexitCode: {:?}\ncwd: {}\ntimedOut: {}\nbackground: {}\npid: {:?}\nstdout:\n{}\nstderr:\n{}",
        result.exit_code,
        result.cwd.as_deref().unwrap_or(""),
        result.timed_out,
        result.background,
        result.pid,
        result.stdout,
        result.stderr
    )
}
