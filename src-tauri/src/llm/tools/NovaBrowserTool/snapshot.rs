use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        crate::llm::tools::shared::browser_automation::execute_sync_stub,
        execute_with_app_boxed,
        true,
        None,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "nova_browser_snapshot".into(),
        description: "Return the current visible Nova Browser tab state for this conversation. Use this before clicking or typing. It captures bounded DOM text plus visible interactive elements across the top page and reachable iframes; use returned refs for click/type. Do not confuse this with external MCP browser tools.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "timeout_ms": { "type": "integer", "description": "Optional timeout in milliseconds. Defaults to 30000 and is capped at 60000." }
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
        "snapshot",
        app,
        conversation_id,
        input,
    )
}
