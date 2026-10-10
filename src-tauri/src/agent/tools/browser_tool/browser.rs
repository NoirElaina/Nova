use crate::agent::tools::{app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration};
use crate::provider::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None, ToolDisclosure::Deferred)
}

pub fn tool() -> Tool {
    Tool {
        name: "Browser".into(),
        description: r#"Interact with Nova's built-in web browser to view, navigate, inspect, and automate web pages.

Supported actions:
- `navigate`: Navigate to a URL. Input: `{"action": "navigate", "url": "https://example.com"}`
- `snapshot`: Capture the current DOM tree, visible text, and interactive elements with element refs (`ref`). Input: `{"action": "snapshot"}`
- `click`: Click on an element by `ref` (from snapshot), CSS `selector`, or coordinates `x` and `y`. Input: `{"action": "click", "ref": "e12"}`
- `type`: Type text into an input element. Input: `{"action": "type", "ref": "e12", "text": "hello"}`
- `reset`: Reset the browser state or clear cache. Input: `{"action": "reset", "clear_data": true}`"#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "snapshot", "click", "type", "reset"],
                    "description": "The browser action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "The URL to navigate to (required for navigate)"
                },
                "ref": {
                    "type": "string",
                    "description": "Element reference ID from snapshot (for click/type)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for click"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type into an element (required for type)"
                },
                "x": {
                    "type": "number",
                    "description": "X coordinate for click"
                },
                "y": {
                    "type": "number",
                    "description": "Y coordinate for click"
                },
                "clear_data": {
                    "type": "boolean",
                    "description": "Whether to clear browser cache and storage during reset"
                }
            },
            "required": ["action"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(app, conversation_id, input).await })
}

async fn execute_async(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing 'action' parameter"))?;

    let result = crate::services::browser_sessions::run_command(
        &app,
        conversation_id.as_deref(),
        action,
        input.clone(),
        None,
    )
    .await;

    if result.get("ok").and_then(Value::as_bool) == Some(false) {
        let err_msg = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Browser command failed");
        return Err(ToolFailure::new(err_msg.to_string()));
    }

    let formatted = serde_json::to_string_pretty(&result)
        .unwrap_or_else(|_| result.to_string());

    Ok(ToolOutcome::text(formatted))
}
