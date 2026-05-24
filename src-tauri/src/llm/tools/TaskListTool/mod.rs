use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 task_list，声明它是只读同步工具，用于列出当前会话全部任务。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true, None)
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

// 直接读取当前内存任务表，并把所有任务打包成 JSON 数组返回。
pub fn execute(_input: Value) -> String {
    json!({ "ok": false, "error": "task_list requires conversation-aware execution" }).to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let tasks = task_store::list(conversation_id.as_deref());
        json!({ "ok": true, "tasks": tasks }).to_string()
    })
}
