use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::services::mcp_tools::build_mcp_tool_name;
use crate::llm::tools::{ToolExecResult, ToolFailure, ToolOutcome};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

pub fn is_needs_user_input_payload(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|s| s == "needs_user_input")
        })
        .unwrap_or(false)
}

fn permission_wait_timeout_ms() -> u64 {
    std::env::var("NOVA_PERMISSION_WAIT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(15 * 60 * 1000)
}

pub async fn await_permission_and_recheck(
    app: &AppHandle,
    conversation_id: Option<&str>,
    tool_name: &str,
    permission_input: &Value,
    request_id: String,
    payload: String,
) -> Result<(), String> {
    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "permission-request".into(),
            text: Some(payload),
            tool_use_id: Some(request_id.clone()),
            tool_use_name: Some(tool_name.to_string()),
            tool_use_input: None,
            tool_result: None,
            tool_is_error: None,
            token_usage: None,
            stop_reason: None,
            turn_state: Some("awaiting_permission".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .map_err(|e| {
        format!(
            "Permission request failed for '{}': unable to notify frontend ({})",
            tool_name, e
        )
    })?;

    let decision = crate::llm::utils::permissions::await_permission_decision(
        conversation_id,
        &request_id,
        permission_wait_timeout_ms(),
    )
    .await
    .map_err(|e| format!("Permission request failed for '{}': {}", tool_name, e))?;

    if matches!(
        decision,
        crate::llm::utils::permissions::PermissionAction::DenyOnce
    ) {
        return Err(format!("Permission denied by user for '{}'", tool_name));
    }

    match crate::llm::utils::permissions::enforce_tool_permission(
        app,
        conversation_id,
        tool_name,
        permission_input,
    ) {
        crate::llm::utils::permissions::PermissionEnforcement::Allow => Ok(()),
        crate::llm::utils::permissions::PermissionEnforcement::Deny(e) => Err(e),
        crate::llm::utils::permissions::PermissionEnforcement::AskUser { .. } => Err(format!(
            "Permission decision for '{}' is still pending",
            tool_name
        )),
    }
}

pub(crate) async fn call_mcp_tool_with_nested_permission(
    app: &AppHandle,
    conversation_id: Option<&str>,
    server_name: String,
    tool_name: String,
    arguments: Value,
) -> ToolExecResult {
    let resolved_tool_name = build_mcp_tool_name(&server_name, &tool_name);

    match crate::llm::utils::permissions::enforce_tool_permission(
        app,
        conversation_id,
        &resolved_tool_name,
        &arguments,
    ) {
        crate::llm::utils::permissions::PermissionEnforcement::Allow => {}
        crate::llm::utils::permissions::PermissionEnforcement::Deny(e) => {
            return Err(ToolFailure::permission_denied(e));
        }
        crate::llm::utils::permissions::PermissionEnforcement::AskUser {
            request_id,
            payload,
        } => {
            if let Err(e) = await_permission_and_recheck(
                app,
                conversation_id,
                &resolved_tool_name,
                &arguments,
                request_id,
                payload,
            )
            .await
            {
                return Err(ToolFailure::permission_denied(e));
            }
        }
    }

    match crate::command::mcp::call_mcp_tool(app.clone(), server_name, tool_name, arguments).await {
        Ok(v) if v.get("isError").and_then(|value| value.as_bool()) == Some(true) => {
            Err(ToolFailure::mcp(v.to_string()))
        }
        Ok(v) => Ok(ToolOutcome::json(v)),
        Err(e) => Err(ToolFailure::mcp(e)),
    }
}
