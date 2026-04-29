use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::process::Command;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "web_fetch".into(),
        description: "Fetch the main textual content of a web page URL.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "HTTP/HTTPS URL to fetch" }
            },
            "required": ["url"]
        }),
    }
}

fn truncate(s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }

    let mut boundary = max_bytes;
    while !s.is_char_boundary(boundary) {
        boundary -= 1;
    }

    format!("{}\n...(truncated)", &s[..boundary])
}

pub fn execute(input: Value) -> String {
    let url = match input.get("url").and_then(|v| v.as_str()) {
        Some(v) if v.starts_with("http://") || v.starts_with("https://") => v,
        _ => return "Error: Missing or invalid 'url' argument".into(),
    };

    #[cfg(target_os = "windows")]
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(Invoke-WebRequest -UseBasicParsing -Uri '{}' -TimeoutSec 20).Content",
                url.replace('"', "")
            ),
        ])
        .output();

    #[cfg(not(target_os = "windows"))]
    let out = Command::new("curl")
        .args(["-L", "--max-time", "20", url])
        .output();

    match out {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if output.status.success() {
                if stdout.trim().is_empty() {
                    "(fetched successfully but content is empty)".into()
                } else {
                    truncate(stdout, 12000)
                }
            } else {
                format!("Error fetching url: {}", stderr)
            }
        }
        Err(e) => format!("Failed to fetch url: {}", e),
    }
}
