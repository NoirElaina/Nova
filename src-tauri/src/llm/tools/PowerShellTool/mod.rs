use crate::llm::tools::{app_tool, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        execute_sync_stub,
        execute_with_app_boxed,
        false,
        Some(permission),
    )
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    // PowerShellTool 和 BashTool 一样，由工具自己提供命令级权限描述。
    crate::llm::utils::permissions::describe_shell_command_permission(
        "execute_powershell",
        "PowerShell 命令",
        input,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "execute_powershell".into(),
        description: "Execute a PowerShell command in a conversation-scoped persistent PowerShell 7 session on Windows. The session keeps its working directory and environment between calls. Use background=true for long-running tasks. Interactive TUI programs are not supported.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "PowerShell command text" },
                "timeout_ms": { "type": "integer", "description": "Optional foreground timeout in milliseconds. Defaults to 300000 and is capped at 1800000." },
                "background": { "type": "boolean", "description": "When true, start the command in the background and return its pid immediately." }
            },
            "required": ["command"]
        }),
    }
}

pub fn execute_sync_stub(_input: Value) -> String {
    json!({ "ok": false, "error": "PowerShellTool requires async execution context" }).to_string()
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    // cmd: 用户或模型传入的 PowerShell 命令文本。
    let cmd = match input.get("command").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.to_string(),
        _ => return "Error: Missing 'command' argument".into(),
    };
    let timeout_ms = input.get("timeout_ms").and_then(|value| value.as_u64());
    let background = input
        .get("background")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    #[cfg(target_os = "windows")]
    {
        let workspace_root = match crate::command::workspace::workspace_root_string_for_conversation(
            app,
            conversation_id,
        ) {
            Ok(root) => root,
            Err(error) => return format!("Failed to resolve workspace: {}", error),
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

        return match result {
            Ok(result) if result.cancelled => {
                json!({ "ok": false, "error": "cancelled" }).to_string()
            }
            Ok(result) if result.timed_out => {
                let stderr = result.stderr.trim();
                let stdout = result.stdout.trim();
                if stderr.is_empty() && stdout.is_empty() {
                    "Error: command timed out".to_string()
                } else {
                    format!(
                        "Error: command timed out\nStderr: {}\nStdout: {}",
                        stderr, stdout
                    )
                }
            }
            Ok(result) if result.background => result.stdout,
            Ok(result) if result.exit_code.unwrap_or(1) == 0 => {
                if result.stdout.trim().is_empty() {
                    "(command executed with no output)".into()
                } else {
                    result.stdout
                }
            }
            Ok(result) => format!("Error: {}\nStdout: {}", result.stderr, result.stdout),
            Err(e) => format!("Failed to execute PowerShell command: {}", e),
        };
    }

    #[cfg(not(target_os = "windows"))]
    {
        format!(
            "Error: execute_powershell is only available on Windows. Command was: {}",
            cmd
        )
    }
}
