use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "replace_string_in_file".into(),
        description: "Replace an exact string with a new string in a file.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute path to the file" },
                "old_string": { "type": "string", "description": "The exact string to be replaced" },
                "new_string": { "type": "string", "description": "The string to replace with" }
            },
            "required": ["path", "old_string", "new_string"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    if let (Some(path), Some(old_string), Some(new_string)) = (
        input.get("path").and_then(|v| v.as_str()),
        input.get("old_string").and_then(|v| v.as_str()),
        input.get("new_string").and_then(|v| v.as_str())
    ) {
        match fs::read_to_string(path) {
            Ok(content) => {
                if !content.contains(old_string) {
                    "Error: old_string not found in file".into()
                } else {
                    let new_content = content.replacen(old_string, new_string, 1);
                    match fs::write(path, new_content) {
                        Ok(_) => "Successfully replaced string in file".into(),
                        Err(e) => format!("Error writing file: {}", e),
                    }
                }
            },
            Err(e) => format!("Error reading file: {}", e),
        }
    } else {
        "Error: Missing 'path', 'old_string', or 'new_string' argument".into()
    }
}
