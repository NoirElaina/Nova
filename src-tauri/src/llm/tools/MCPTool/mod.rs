use crate::llm::services::mcp_tools::parse_mcp_tool_name;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "mcp_tool".into(),
        description: "Call a tool exposed by a configured MCP server.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "tool": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["server", "tool"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    let server = input.get("server").and_then(|v| v.as_str()).unwrap_or("");
    let tool = input.get("tool").and_then(|v| v.as_str()).unwrap_or("");
    json!({
        "ok": false,
        "server": server,
        "tool": tool,
        "message": "mcp_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let server_name = input
        .get("server")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let tool_name = input
        .get("tool")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let arguments = input
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if server_name.is_empty() || tool_name.is_empty() {
        return json!({
            "ok": false,
            "error": "mcp_tool requires non-empty 'server' and 'tool' fields"
        })
        .to_string();
    }

    match crate::command::mcp::call_mcp_tool(app.clone(), server_name, tool_name, arguments).await {
        Ok(v) => v.to_string(),
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    }
}

pub(crate) fn dynamic_tool_read_only(name: &str) -> Option<bool> {
    let (_server_name, tool_name) = parse_mcp_tool_name(name)?;
    let tool_lower = tool_name.to_ascii_lowercase();
    Some(
        ["read", "list", "search", "get", "fetch", "glob", "grep"]
            .iter()
            .any(|kw| tool_lower.contains(kw)),
    )
}

pub(crate) fn dynamic_tool_permission_descriptor(
    _name: &str,
    _input: &Value,
) -> Option<ToolPermissionDescriptor> {
    None
}

pub(crate) async fn execute_dynamic_with_app(
    app: &AppHandle,
    name: &str,
    arguments: Value,
) -> Option<String> {
    let (server_name, tool_name) = parse_mcp_tool_name(name)?;
    Some(
        execute_with_app(
            app,
            json!({
                "server": server_name,
                "tool": tool_name,
                "arguments": arguments,
            }),
        )
        .await,
    )
}
