use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "TaskCreate".into(),
        description: "Create a task item (Claude-compatible name).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "subject": { "type": "string" },
                "status": { "type": "string" },
                "notes": { "type": "string" },
                "description": { "type": "string" }
            }
        }),
    }
}

pub fn execute(input: Value) -> String {
    let title = input
        .get("title")
        .or_else(|| input.get("subject"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let Some(title) = title else {
        return "Error: Missing 'title' or 'subject' argument".into();
    };

    let status = input
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("not-started")
        .to_string();

    let notes = input
        .get("notes")
        .or_else(|| input.get("description"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let task = task_store::create(title, status, notes);
    json!({ "ok": true, "task": task }).to_string()
}
