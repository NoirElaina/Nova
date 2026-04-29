use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "TaskStop".into(),
        description: "Stop a running task by id (Claude-compatible name).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "task_id": { "type": ["string", "integer"] },
                "shell_id": { "type": ["string", "integer"] },
                "id": { "type": ["string", "integer"] }
            }
        }),
    }
}

fn parse_task_id(input: &Value) -> Option<u64> {
    for key in ["task_id", "shell_id", "id"] {
        if let Some(v) = input.get(key) {
            if let Some(id) = v.as_u64() {
                return Some(id);
            }
            if let Some(s) = v.as_str() {
                if let Ok(id) = s.trim().parse::<u64>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

pub fn execute(input: Value) -> String {
    let Some(task_id) = parse_task_id(&input) else {
        return "Error: Missing task id (task_id/shell_id/id)".into();
    };

    match task_store::update(task_id, None, Some("stopped".into()), None) {
        Some(task) => json!({
            "ok": true,
            "message": format!("Successfully stopped task: {}", task.id),
            "task_id": task.id.to_string(),
            "task_type": "todo",
            "command": task.title
        })
        .to_string(),
        None => format!("Error: Task id {} not found", task_id),
    }
}
