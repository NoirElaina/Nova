use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::permissions::RiskLevel;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission), ToolDisclosure::Core)
}

/// 终止进程是有副作用的破坏性动作，需要用户确认。
/// 只接受后台作业 id，不接受裸 pid——这样本工具无法用于终止任意进程。
fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let id = input.get("id").and_then(Value::as_str).unwrap_or("");
    Some(ToolPermissionDescriptor {
        signature: format!("bashkill:{}", id),
        preview: format!("终止后台作业 {}", id),
        warning: Some("该作业及其子进程会被强制结束".to_string()),
        risk: RiskLevel::Risky,
    })
}

pub fn tool() -> Tool {
    Tool {
        name: "BashKill".into(),
        description: r#"Stop a background Bash job started with `Bash(run_in_background: true)`.

- `id` (required): the job id returned by the background Bash call (e.g. `bg-1`). Call BashOutput with no id to list jobs and their ids.

Kills the job's whole process tree, then returns the output captured up to that point. Only jobs started by Bash in this app can be targeted; arbitrary pids are not accepted."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Background job id to kill (e.g. \"bg-1\")"
                }
            },
            "required": ["id"]
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
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let id = input
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolFailure::invalid_input("Missing 'id'. Call BashOutput with no id to list background jobs."))?;

    match crate::llm::services::shell_sessions::kill_background_job(id).await {
        Ok(output) => Ok(ToolOutcome::json(
            serde_json::to_value(output).unwrap_or_else(|_| json!({})),
        )),
        Err(error) => Err(ToolFailure::new(error)),
    }
}
