use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

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

pub fn execute(input: Value) -> String {
    let todos = match input.get("todos").and_then(|v| v.as_array()) {
        Some(v) => v,
        None => return "Error: Missing 'todos' array".into(),
    };

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

    let created = task_store::replace_all(items);
    json!({ "ok": true, "tasks": created }).to_string()
}
