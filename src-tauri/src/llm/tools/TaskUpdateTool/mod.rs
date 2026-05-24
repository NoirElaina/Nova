use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 task_update，声明它是写类会话工具，用于更新已有任务。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的工具元数据，要求提供 id，其他字段按需更新。
pub fn tool() -> Tool {
    Tool {
        name: "task_update".into(),
        description: "Update a task item by id.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer" },
                "title": { "type": "string" },
                "status": { "type": "string", "enum": ["not-started", "in-progress", "completed"] },
                "notes": { "type": ["string", "null"] }
            },
            "required": ["id"]
        }),
    }
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_scoped(conversation_id.as_deref(), input) })
}

fn execute_scoped(conversation_id: Option<&str>, input: Value) -> String {
    let id = match input.get("id").and_then(|v| v.as_u64()) {
        Some(v) => v,
        None => return json!({ "ok": false, "error": "Missing 'id' argument" }).to_string(),
    };

    let title = input
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // status: 新状态；未提供时保持原值不变。
    let status = input
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // notes: Some(None) 表示显式清空备注；None 表示本次不改备注字段。
    let notes = if input.get("notes").is_some() {
        Some(input.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string()))
    } else {
        None
    };

    match task_store::update(conversation_id, id, title, status, notes) {
        Some(task) => json!({ "ok": true, "task": task }).to_string(),
        None => json!({ "ok": false, "error": format!("Task id {} not found", id) }).to_string(),
    }
}
