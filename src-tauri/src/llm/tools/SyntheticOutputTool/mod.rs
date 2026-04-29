use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "StructuredOutput".into(),
        description: "Return structured JSON output as the final machine-readable result.".into(),
        input_schema: json!({
            "type": "object",
            "description": "Arbitrary structured JSON object that will be returned to the caller as-is."
        }),
    }
}

pub fn execute(input: Value) -> String {
    json!({
        "ok": true,
        "message": "Structured output provided successfully",
        "structured_output": input
    })
    .to_string()
}
