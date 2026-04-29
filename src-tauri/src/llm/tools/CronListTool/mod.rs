use crate::llm::tools::shared::cron_store::list_jobs;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "CronList".into(),
        description: "List scheduled cron jobs for the current session and durable store.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "CronList requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(app: &AppHandle, _input: Value) -> String {
    match list_jobs(app) {
        Ok(jobs) => {
            let list = jobs
                .into_iter()
                .map(|job| {
                    json!({
                        "id": job.id,
                        "cron": job.cron,
                        "humanSchedule": job.cron,
                        "prompt": job.prompt,
                        "conversationId": job.conversation_id,
                        "recurring": job.recurring,
                        "durable": job.durable,
                        "createdAt": job.created_at
                    })
                })
                .collect::<Vec<_>>();

            json!({ "ok": true, "jobs": list }).to_string()
        }
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    }
}
