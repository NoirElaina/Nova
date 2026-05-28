use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "nova_browser_reset".into(),
        description: "Reset the conversation's Nova Browser window. This clears the current page state in Nova's built-in browser; optionally clears browsing data when clear_data is true.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "clear_data": { "type": "boolean", "description": "Clear browser cookies/cache/local data before reset. Defaults to false." },
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
        "reset",
        app,
        conversation_id,
        input,
    )
}
