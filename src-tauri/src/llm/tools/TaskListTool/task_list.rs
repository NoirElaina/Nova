use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 task_list，声明它是只读会话工具，用于列出当前会话全部任务。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回暴露给模型的工具元数据；这个工具不需要额外输入字段。
pub fn tool() -> Tool {
    Tool {
        name: "task_list".into(),
        description: "List all session tasks.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let tasks = task_store::list(conversation_id.as_deref());
        Ok(ToolOutcome::json(json!({ "ok": true, "tasks": tasks })))
    })
}
