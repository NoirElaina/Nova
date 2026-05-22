use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        crate::llm::tools::shared::browser_automation::execute_sync_stub,
        execute_with_app_boxed,
        false,
        None,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "nova_browser_navigate".into(),
        description: "Navigate the conversation's Nova Browser window to a URL or search query. Nova can open or focus the browser window automatically; this operates Nova's built-in browser, not external MCP browser tools.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL or search query to open in the built-in Nova Browser." },
                "timeout_ms": { "type": "integer", "description": "Optional timeout in milliseconds. Defaults to 15000 and is capped at 60000." }
            },
            "required": ["url"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    crate::llm::tools::shared::browser_automation::run_browser_action_boxed(
        "navigate",
        app,
        conversation_id,
        input,
    )
}
