use crate::llm::tools::{app_tool, AppExecuteFuture, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 exit_plan_mode，声明它是无权限要求的同步状态切换工具。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的工具元数据，告诉模型这个工具用于退出 plan 模式。
pub fn tool() -> Tool {
    Tool {
        name: "exit_plan_mode".into(),
        description: "Exit plan mode after the planning phase is complete and resume normal implementation work.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Optional summary of the agreed plan"
                }
            }
        }),
    }
}

// 读取可选 summary，并返回一个 plan_mode_change payload 给前端切换模式。
fn execute_local(input: Value) -> String {
    let summary = input
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    json!({
        "type": "plan_mode_change",
        "mode": "default",
        "summary": summary,
        "message": "Exited plan mode. You may now implement the approved plan."
    })
    .to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { Ok(ToolOutcome::text(execute_local(input))) })
}
