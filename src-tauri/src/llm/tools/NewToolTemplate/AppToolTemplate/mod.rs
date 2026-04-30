use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "app_tool".into(),
        description: "A template for tools that need AppHandle or async work.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Example input field"
                }
            },
            "required": ["input"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    json!({
        "ok": false,
        "message": "app_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app.",
        "input": input
    })
    .to_string()
}

pub async fn execute_with_app(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> String {
    let value = match input.get("input").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'input'" }).to_string(),
    };

    json!({
        "ok": true,
        "input": value,
        "message": "Replace this with your real AppHandle-aware logic."
    })
    .to_string()
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}
