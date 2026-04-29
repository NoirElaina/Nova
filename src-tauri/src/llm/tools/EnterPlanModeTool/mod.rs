use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "enter_plan_mode".into(),
        description: "Switch Nova into plan mode for read-first exploration and implementation planning before making code changes.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Optional short summary of what should be planned"
                }
            }
        }),
    }
}

pub fn execute(input: Value) -> String {
    let goal = input
        .get("goal")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    json!({
        "type": "plan_mode_change",
        "mode": "plan",
        "goal": goal,
        "message": "Entered plan mode. Focus on exploration, trade-offs, and a concrete plan before editing files."
    })
    .to_string()
}
