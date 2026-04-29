use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "mcp_auth".into(),
        description: "Manage MCP connection/auth lifecycle: status, reload, enable/disable server, list tools, and probe tool access.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "reload_all", "enable", "disable", "list_tools", "probe_tool"]
                },
                "server": { "type": "string" },
                "tool": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["action"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("unknown");
    json!({
        "ok": false,
        "action": action,
        "message": "mcp_auth requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> String {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "status" => match crate::command::mcp::get_mcp_server_statuses(app.clone()).await {
            Ok(statuses) => json!({
                "ok": true,
                "action": "status",
                "servers": statuses
            })
            .to_string(),
            Err(e) => json!({ "ok": false, "error": e }).to_string(),
        },
        "reload_all" => {
            if let Err(e) = crate::command::mcp::reload_all_mcp_servers(app.clone()).await {
                return json!({ "ok": false, "error": e }).to_string();
            }
            match crate::command::mcp::get_mcp_server_statuses(app.clone()).await {
                Ok(statuses) => json!({
                    "ok": true,
                    "action": "reload_all",
                    "servers": statuses
                })
                .to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        "enable" | "disable" => {
            let Some(server_name) = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return json!({
                    "ok": false,
                    "error": "mcp_auth action requires non-empty 'server'"
                })
                .to_string();
            };

            let enabled = action == "enable";
            match crate::command::mcp::set_mcp_server_enabled(
                app.clone(),
                server_name.to_string(),
                enabled,
            )
            .await
            {
                Ok(()) => json!({
                    "ok": true,
                    "action": action,
                    "server": server_name,
                    "enabled": enabled
                })
                .to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        "list_tools" => {
            let Some(server_name) = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return json!({
                    "ok": false,
                    "error": "mcp_auth list_tools requires non-empty 'server'"
                })
                .to_string();
            };

            match crate::command::mcp::list_mcp_tools(app.clone(), server_name.to_string()).await {
                Ok(tools) => json!({
                    "ok": true,
                    "action": "list_tools",
                    "server": server_name,
                    "tools": tools
                })
                .to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        "probe_tool" => {
            let server_name = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            let tool_name = input
                .get("tool")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            let arguments = input
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            if server_name.is_empty() || tool_name.is_empty() {
                return json!({
                    "ok": false,
                    "error": "mcp_auth probe_tool requires non-empty 'server' and 'tool'"
                })
                .to_string();
            }

            crate::llm::tools::shared::permission_runtime::call_mcp_tool_with_nested_permission(
                app,
                conversation_id,
                server_name,
                tool_name,
                arguments,
            )
            .await
        }
        _ => json!({
            "ok": false,
            "error": "mcp_auth action must be one of: status, reload_all, enable, disable, list_tools, probe_tool"
        })
        .to_string(),
    }
}
