use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "read_only_tool".into(),
        description: "A simple read-only tool template.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The input to inspect"
                }
            },
            "required": ["query"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'query'" }).to_string(),
    };

    json!({
        "ok": true,
        "query": query,
        "message": "Replace this with your read-only logic."
    })
    .to_string()
}
