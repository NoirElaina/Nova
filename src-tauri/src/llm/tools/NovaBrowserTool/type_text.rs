use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "nova_browser_type".into(),
        description: "Type text into an element inside the conversation's Nova Browser window. Prefer the ref returned by nova_browser_snapshot because it also works for iframe elements. Otherwise use selector, active element, or x/y to focus first.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to type." },
                "ref": { "type": "string", "description": "Element ref returned by nova_browser_snapshot, including iframe refs like f2_el3." },
                "selector": { "type": "string", "description": "CSS selector for the editable element." },
                "x": { "type": "number", "description": "Viewport x coordinate to focus before typing." },
                "y": { "type": "number", "description": "Viewport y coordinate to focus before typing." },
                "clear": { "type": "boolean", "description": "Clear existing value before typing. Defaults to false." },
                "submit": { "type": "boolean", "description": "Submit with Enter after typing. Defaults to false." },
                "timeout_ms": { "type": "integer", "description": "Optional timeout in milliseconds. Defaults to 15000 and is capped at 60000." }
            },
            "required": ["text"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    crate::llm::tools::shared::browser_automation::run_browser_action_boxed(
        "type",
        app,
        conversation_id,
        input,
    )
}
