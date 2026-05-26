use crate::llm::tools::shared::cron_store::remove_job;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把删除计划任务的 async 逻辑包装成统一 future。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 CronDelete 的注册信息。
// `read_only=false`，因为它会删除已有的计划任务。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回 CronDelete 暴露给模型的元数据。
pub fn tool() -> Tool {
    Tool {
        name: "CronDelete".into(),
        description: "Delete a scheduled cron job by id.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Job id returned by CronCreate" }
            },
            "required": ["id"]
        }),
    }
}

// 按 `id` 删除一个计划任务。
// `id` 是 CronCreate 返回的任务标识，找不到时会返回明确错误。
async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let id = match input.get("id").and_then(|v| v.as_str()).map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => return Err(ToolFailure::invalid_input("CronDelete requires non-empty 'id'")),
    };

    match remove_job(app, id) {
        Ok(true) => Ok(ToolOutcome::json(json!({ "ok": true, "id": id }))),
        Ok(false) => Err(ToolFailure::new(format!(
            "No scheduled job with id '{}'",
            id
        ))),
        Err(e) => Err(ToolFailure::new(e)),
    }
}
