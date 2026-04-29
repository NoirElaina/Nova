use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::process::Command;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "execute_bash".into(),
        description: "Execute a bash or powershell command on the host machine.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to execute" }
            },
            "required": ["command"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
        #[cfg(target_os = "windows")]
        let out = Command::new("powershell").args(["-Command", cmd]).output();

        #[cfg(not(target_os = "windows"))]
        let out = Command::new("sh").arg("-c").arg(cmd).output();

        match out {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if output.status.success() {
                    stdout
                } else {
                    format!("Error: {}\nStdout: {}", stderr, stdout)
                }
            }
            Err(e) => format!("Failed to execute command: {}", e),
        }
    } else {
        "Error: Missing 'command' argument".into()
    }
}
