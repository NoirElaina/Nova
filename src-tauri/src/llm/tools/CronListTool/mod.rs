use crate::llm::tools::shared::cron_store::list_jobs;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把列出计划任务的 async 逻辑包装成统一 future。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 CronList 的注册信息。
// `read_only=true`，因为它只读取任务列表，不改动任何状态。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回 CronList 暴露给模型的元数据。
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

// 读取当前会话和持久化存储里的所有计划任务。
// `jobs` 是合并后的任务列表，最后会统一转成 JSON 数组返回给模型。
async fn execute_with_app(app: &AppHandle, _input: Value) -> Result<ToolOutcome, ToolFailure> {
    match list_jobs(app) {
        Ok(jobs) => {
            // list: 返回给模型的轻量序列化结果，只保留工具协议需要的字段。
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

            Ok(ToolOutcome::json(json!({ "ok": true, "jobs": list })))
        }
        Err(e) => Err(ToolFailure::new(e)),
    }
}
