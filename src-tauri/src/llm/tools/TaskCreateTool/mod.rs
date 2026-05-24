use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 task_create，声明它是写类同步工具，用于创建当前会话任务。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的工具元数据，要求至少提供 title。
pub fn tool() -> Tool {
    Tool {
        name: "task_create".into(),
        description: "Create a task item for the current session.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "status": { "type": "string", "enum": ["not-started", "in-progress", "completed"] },
                "notes": { "type": "string" }
            },
            "required": ["title"]
        }),
    }
}

// 从输入里提取 title/status/notes，并在内存任务表里创建一条新任务。
pub fn execute(_input: Value) -> String {
    json!({ "ok": false, "error": "task_create requires conversation-aware execution" })
        .to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_scoped(conversation_id.as_deref(), input) })
}

fn execute_scoped(conversation_id: Option<&str>, input: Value) -> String {
    let title = match input.get("title").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return json!({ "ok": false, "error": "Missing 'title' argument" }).to_string(),
    };

    let status = input
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("not-started")
        .to_string();

    // notes: 可选备注，缺省时保持为 None。
    let notes = input.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string());

    let task = task_store::create(conversation_id, title, status, notes);
    json!({ "ok": true, "task": task }).to_string()
}
