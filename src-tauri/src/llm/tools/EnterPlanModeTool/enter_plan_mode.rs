use crate::llm::tools::{app_tool, AppExecuteFuture, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 enter_plan_mode，声明它是无权限要求的同步状态切换工具。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的工具元数据，告诉模型这个工具用于进入 plan 模式。
pub fn tool() -> Tool {
    Tool {
        name: "enter_plan_mode".into(),
        description: "Switch Nova into plan mode for read-first exploration and implementation planning before making code changes.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Optional short summary of what should be planned"
                }
            }
        }),
    }
}

// 读取可选 goal，并返回一个 plan_mode_change payload 给前端切换模式。
fn execute_local(input: Value) -> String {
    let goal = input
        .get("goal")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    json!({
        "type": "plan_mode_change",
        "mode": "plan",
        "goal": goal,
        "message": "Entered plan mode. Focus on exploration, trade-offs, and a concrete plan before editing files."
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
