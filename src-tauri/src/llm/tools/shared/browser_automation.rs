use crate::llm::tools::{AppExecuteFuture, ToolFailure, ToolOutcome};
use serde_json::Value;
use tauri::AppHandle;

pub(crate) fn run_browser_action_boxed(
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
) -> Result<ToolOutcome, ToolFailure> {
    let timeout_ms = input.get("timeout_ms").and_then(|value| value.as_u64());
    let value = crate::llm::services::browser_sessions::run_command(
        &app,
        conversation_id.as_deref(),
        action,
        input,
        timeout_ms,
    )
    .await;

    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        let message = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("browser command failed")
            .to_string();
        return Err(ToolFailure::new(message));
    }

    Ok(ToolOutcome::json(value))
}
