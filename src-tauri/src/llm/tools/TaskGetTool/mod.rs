use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "TaskGet".into(),
        description: "Retrieve a task by ID (Claude-compatible name).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "taskId": { "type": "string" },
                "id": { "type": ["integer", "string"] }
            }
        }),
    }
}

fn parse_task_id(input: &Value) -> Option<u64> {
    if let Some(id) = input.get("id") {
        if let Some(v) = id.as_u64() {
            return Some(v);
        }
        if let Some(s) = id.as_str() {
            return s.trim().parse::<u64>().ok();
        }
    }

    input
        .get("taskId")
        .and_then(|v| v.as_str())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

pub fn execute(input: Value) -> String {
    let Some(task_id) = parse_task_id(&input) else {
        return "Error: Missing 'taskId' or numeric 'id'".into();
    };

    let task = task_store::get(task_id);
    json!({ "ok": true, "task": task }).to_string()
}
