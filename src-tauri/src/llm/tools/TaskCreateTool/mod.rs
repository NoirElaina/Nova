use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

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

pub fn execute(input: Value) -> String {
    let title = match input.get("title").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return "Error: Missing 'title' argument".into(),
    };

    let status = input
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("not-started")
        .to_string();

    let notes = input.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string());

    let task = task_store::create(title, status, notes);
    json!({ "ok": true, "task": task }).to_string()
}
