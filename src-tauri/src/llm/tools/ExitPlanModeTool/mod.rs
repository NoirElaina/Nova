use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "exit_plan_mode".into(),
        description: "Exit plan mode after the planning phase is complete and resume normal implementation work.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Optional summary of the agreed plan"
                }
            }
        }),
    }
}

pub fn execute(input: Value) -> String {
    let summary = input
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    json!({
        "type": "plan_mode_change",
        "mode": "default",
        "summary": summary,
        "message": "Exited plan mode. You may now implement the approved plan."
    })
    .to_string()
}
