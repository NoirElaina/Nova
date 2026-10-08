use crate::agent::capabilities::mcp_tools::build_mcp_tool_name;
use crate::agent::tools::{ToolExecResult, ToolFailure, ToolOutcome};
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
    let _ = app.emit(
        "agent-event",
        crate::agent::events::AgentDomainEvent::PermissionRequested {
            turn_id: conversation_id.unwrap_or_default().to_string(),
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            payload: payload.clone(),
        },
    );

    let decision = crate::agent::permissions::await_permission_decision(
        conversation_id,
        &request_id,
        permission_wait_timeout_ms(),
    )
    .await
    .map_err(|e| format!("Permission request failed for '{}': {}", tool_name, e))?;

    if matches!(
        decision,
        crate::agent::permissions::PermissionAction::DenyOnce
    ) {
        return Err(format!("Permission denied by user for '{}'", tool_name));
    }

    match crate::agent::permissions::enforce_tool_permission(
        app,
        conversation_id,
        tool_name,
        permission_input,
    ) {
        crate::agent::permissions::PermissionEnforcement::Allow => Ok(()),
        crate::agent::permissions::PermissionEnforcement::Deny(e) => Err(e),
        crate::agent::permissions::PermissionEnforcement::AskUser { .. } => Err(format!(
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

    match crate::agent::permissions::enforce_tool_permission(
        app,
        conversation_id,
        &resolved_tool_name,
        &arguments,
    ) {
        crate::agent::permissions::PermissionEnforcement::Allow => {}
        crate::agent::permissions::PermissionEnforcement::Deny(e) => {
            return Err(ToolFailure::permission_denied(e));
        }
        crate::agent::permissions::PermissionEnforcement::AskUser {
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

    match crate::agent::capabilities::mcp::call_mcp_tool(app.clone(), server_name, tool_name, arguments).await {
        Ok(v) if v.get("isError").and_then(|value| value.as_bool()) == Some(true) => {
            Err(ToolFailure::mcp(v.to_string()))
        }
        Ok(v) => Ok(ToolOutcome::json(v)),
        Err(e) => Err(ToolFailure::mcp(e)),
    }
}
