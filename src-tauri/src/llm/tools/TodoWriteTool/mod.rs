use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 todo_write，声明它是写类会话工具，会整体替换当前任务列表。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的工具元数据，要求传入 todos 数组。
pub fn tool() -> Tool {
    Tool {
        name: "todo_write".into(),
        description: "Replace the current task list with a provided todo array.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            "status": { "type": "string", "enum": ["not-started", "in-progress", "completed"] },
                            "notes": { "type": "string" }
                        },
                        "required": ["title", "status"]
                    }
                }
            },
            "required": ["todos"]
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
    let todos = match input.get("todos").and_then(|v| v.as_array()) {
        Some(v) => v,
        None => return json!({ "ok": false, "error": "Missing 'todos' array" }).to_string(),
    };

    // items: replace_all 需要的内部任务元组列表。
    let mut items = Vec::new();
    for todo in todos {
        let title = match todo.get("title").and_then(|v| v.as_str()) {
            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
            _ => continue,
        };

        let status = todo
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("not-started")
            .to_string();

        let notes = todo.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string());
        items.push((title, status, notes));
    }

    let created = task_store::replace_all(conversation_id, items);
    json!({ "ok": true, "tasks": created }).to_string()
}
