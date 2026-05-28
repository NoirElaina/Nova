use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "nova_browser_click".into(),
        description: "Click an element inside the conversation's Nova Browser window. Prefer the ref returned by nova_browser_snapshot (for example f1_el12 or iframe refs like f2_el3). Otherwise use selector or x/y viewport coordinates.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "Element ref returned by nova_browser_snapshot, for example f1_el12 or iframe refs like f2_el3." },
                "selector": { "type": "string", "description": "CSS selector to click." },
                "x": { "type": "number", "description": "Viewport x coordinate to click when selector is not available." },
                "y": { "type": "number", "description": "Viewport y coordinate to click when selector is not available." },
                "timeout_ms": { "type": "integer", "description": "Optional timeout in milliseconds. Defaults to 15000 and is capped at 60000." }
            }
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    crate::llm::tools::shared::browser_automation::run_browser_action_boxed(
        "click",
        app,
        conversation_id,
        input,
    )
}
