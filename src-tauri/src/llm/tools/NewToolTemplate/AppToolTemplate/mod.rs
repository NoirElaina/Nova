use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
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

async fn execute_with_app(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let value = match input.get("input").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return Err(ToolFailure::invalid_input("Missing 'input'")),
    };

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "input": value,
        "message": "Replace this with your real AppHandle-aware logic."
    })))
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}
