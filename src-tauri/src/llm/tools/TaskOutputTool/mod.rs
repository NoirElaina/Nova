use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "TaskOutput".into(),
        description: "Return task output-style summary by task id (Claude-compatible name).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "task_id": { "type": ["string", "integer"] },
                "taskId": { "type": ["string", "integer"] },
                "id": { "type": ["string", "integer"] }
            }
        }),
    }
}

fn parse_task_id(input: &Value) -> Option<u64> {
    for key in ["task_id", "taskId", "id"] {
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
        return "Error: Missing task id (task_id/taskId/id)".into();
    };

    let Some(task) = task_store::get(task_id) else {
        return json!({ "ok": true, "retrieval_status": "not_found", "task": Value::Null }).to_string();
    };

    let output = format!(
        "Task #{}\nTitle: {}\nStatus: {}\nNotes: {}",
        task.id,
        task.title,
        task.status,
        task.notes.clone().unwrap_or_else(|| "(none)".into())
    );

    json!({
        "ok": true,
        "retrieval_status": "success",
        "task": {
            "task_id": task.id.to_string(),
            "task_type": "todo",
            "status": task.status,
            "description": task.title,
            "output": output
        }
    })
    .to_string()
}
