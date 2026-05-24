use crate::llm::tools::{app_tool, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 返回 BashTool 的注册信息。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission))
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    crate::llm::utils::permissions::describe_shell_command_permission(
        "execute_bash",
        "终端命令",
        input,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "execute_bash".into(),
        description: "Execute a shell command in a conversation-scoped persistent shell session. The session keeps its working directory and environment between calls. Use background=true for long-running tasks. Interactive TUI programs are not supported.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to execute" },
                "timeout_ms": { "type": "integer", "description": "Optional foreground timeout in milliseconds. Defaults to 300000 and is capped at 1800000." },
                "background": { "type": "boolean", "description": "When true, start the command in the background and return its pid immediately." }
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

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    let cmd = match input.get("command").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.to_string(),
        _ => return json!({ "ok": false, "error": "Missing 'command' argument" }).to_string(),
    };
    let timeout_ms = input.get("timeout_ms").and_then(|value| value.as_u64());
    let background = input
        .get("background")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let workspace_root =
        match crate::command::workspace::workspace_root_string_for_conversation(app, conversation_id)
        {
            Ok(root) => root,
            Err(error) => {
                return json!({
                    "ok": false,
                    "error": format!("Failed to resolve workspace: {}", error)
                })
                .to_string()
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
        Ok(result) if result.cancelled => json!({ "ok": false, "error": "cancelled" }).to_string(),
        Ok(result) if result.timed_out => json!({
            "ok": false,
            "error": "command timed out",
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exitCode": result.exit_code,
            "cwd": result.cwd,
            "timedOut": true,
            "background": result.background,
            "pid": result.pid
        })
        .to_string(),
        Ok(result) => {
            let ok = result.exit_code.unwrap_or(1) == 0;
            json!({
                "ok": ok,
                "error": if ok { Value::Null } else { json!("command exited with non-zero status") },
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exitCode": result.exit_code,
                "cwd": result.cwd,
                "timedOut": result.timed_out,
                "background": result.background,
                "pid": result.pid
            })
            .to_string()
        }
        Err(error) => json!({
            "ok": false,
            "error": format!("Failed to execute command: {}", error)
        })
        .to_string(),
    }
}
