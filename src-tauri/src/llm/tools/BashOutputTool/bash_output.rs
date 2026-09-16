use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    // 只读：读日志文件与进程状态，不修改任何东西。
    app_tool(tool, execute_with_app_boxed, true, None, ToolDisclosure::Core)
}

pub fn tool() -> Tool {
    Tool {
        name: "BashOutput".into(),
        description: r#"Read the output of a background Bash job started with `Bash(run_in_background: true)`.

- `id`: the job id returned by the background Bash call (e.g. `bg-1`). Omit it to list all background jobs for this conversation with their id, pid, command and running state.
- `tail_lines`: how many trailing lines to return per stream. Default 200, max 5000.

Returns `running`, `finished` and `exitCode`, so you can tell "still working" from "succeeded" from "failed". `stdout`/`stderr` hold the most recent output, buffered in memory since the job started (not a delta — call again later to see more). Only the last 512 KB per stream is retained, and what is returned is capped at the last 200 lines / 12000 characters, keeping the tail so recent errors are always visible.

Every background job has a maximum lifetime (`ttlMs`, default 30 min) and is killed automatically when it expires; `remainingMs` shows what is left. Use this to poll a dev server or a long build: start it in the background, then call BashOutput on its id until `running` is false or the output shows what you need. Pair with BashKill to stop it early."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Background job id (e.g. \"bg-1\"). Omit to list all background jobs."
                },
                "tail_lines": {
                    "type": "integer",
                    "description": "Number of trailing lines to return per stream. Default 200, max 5000."
                }
            }
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
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let id = input
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tail_lines = input
        .get("tail_lines")
        .and_then(Value::as_u64)
        .map(|value| value as usize);

    let Some(id) = id else {
        let jobs = crate::llm::services::shell_sessions::list_background_jobs_for_conversation(
            conversation_id,
        );
        if jobs.is_empty() {
            return Ok(ToolOutcome::text(
                "No background jobs for this conversation. Start one with Bash(run_in_background: true); the returned \"id\" is the handle used by BashOutput and BashKill.",
            ));
        }
        return Ok(ToolOutcome::json(json!({ "jobs": jobs })));
    };

    match crate::llm::services::shell_sessions::read_background_output(id, None, tail_lines) {
        Ok(output) => Ok(ToolOutcome::json(
            serde_json::to_value(output).unwrap_or_else(|_| json!({})),
        )),
        Err(error) => Err(ToolFailure::new(error)),
    }
}
