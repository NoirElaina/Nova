use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "read_file".into(),
        description: "Read the content of a file from the host machine.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute path to the file" }
            },
            "required": ["path"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
        fs::read_to_string(path).unwrap_or_else(|e| format!("Error reading file: {}", e))
    } else {
        "Error: Missing 'path' argument".into()
    }
}
