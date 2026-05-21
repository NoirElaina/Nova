use crate::llm::tools::AppExecuteFuture;
use serde_json::{json, Value};
use tauri::AppHandle;

pub fn execute_sync_stub(_input: Value) -> String {
    json!({
        "ok": false,
        "error": "Nova browser tools require app execution context",
    })
    .to_string()
}

pub fn run_browser_action_boxed(
    action: &'static str,
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { run_browser_action(action, app, conversation_id, input).await })
}

async fn run_browser_action(
    action: &'static str,
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> String {
    let timeout_ms = input.get("timeout_ms").and_then(|value| value.as_u64());
    crate::llm::services::browser_sessions::run_command(
        &app,
        conversation_id.as_deref(),
        action,
        input,
        timeout_ms,
    )
    .await
    .to_string()
}
