use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "TaskUpdate".into(),
        description: "Update a task by id (Claude-compatible name).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "taskId": { "type": ["string", "integer"] },
                "id": { "type": ["string", "integer"] },
                "title": { "type": "string" },
                "subject": { "type": "string" },
                "status": { "type": "string" },
                "notes": { "type": ["string", "null"] },
                "description": { "type": ["string", "null"] }
            }
        }),
    }
}

fn parse_task_id(input: &Value) -> Option<u64> {
    if let Some(id) = input.get("id").or_else(|| input.get("taskId")) {
        if let Some(v) = id.as_u64() {
            return Some(v);
        }
        if let Some(s) = id.as_str() {
            return s.trim().parse::<u64>().ok();
        }
    }
    None
}

pub fn execute(input: Value) -> String {
    let Some(task_id) = parse_task_id(&input) else {
        return "Error: Missing 'id' or 'taskId'".into();
    };

    let title = input
        .get("title")
        .or_else(|| input.get("subject"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let status = input
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let notes = if input.get("notes").is_some() || input.get("description").is_some() {
        Some(
            input
                .get("notes")
                .or_else(|| input.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        )
    } else {
        None
    };

    match task_store::update(task_id, title, status, notes) {
        Some(task) => json!({ "ok": true, "task": task }).to_string(),
        None => format!("Error: Task id {} not found", task_id),
    }
}
