use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::process::Command;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "grep_search".into(),
        description: "Search for a pattern in files within a directory.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "The pattern to search for" },
                "path": { "type": "string", "description": "Directory to search in" }
            },
            "required": ["pattern", "path"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    if let (Some(pattern), Some(path)) = (
        input.get("pattern").and_then(|v| v.as_str()),
        input.get("path").and_then(|v| v.as_str())
    ) {
        #[cfg(target_os = "windows")]
        let out = Command::new("powershell").args(["-Command", &format!("Select-String -Path '{}' -Pattern '{}' -Recurse", path, pattern)]).output();

        #[cfg(not(target_os = "windows"))]
        let out = Command::new("grep").args(["-rni", pattern, path]).output();

        match out {
            Ok(output) => {
                let result = String::from_utf8_lossy(&output.stdout).to_string();
                if result.is_empty() {
                    "No matches found".into()
                } else {
                    if result.len() > 10000 {
                        format!("{}...\n(Result truncated)", &result[..10000])
                    } else {
                        result
                    }
                }
            }
            Err(e) => format!("Failed to search: {}", e),
        }
    } else {
        "Error: Missing 'pattern' or 'path' argument".into()
    }
}
