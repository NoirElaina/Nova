use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
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

fn execute_local(input: Value) -> String {
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

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_local(input) })
}
