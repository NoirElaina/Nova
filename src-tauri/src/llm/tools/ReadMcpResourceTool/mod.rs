use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
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
    app_tool(tool, execute, execute_with_app_boxed, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "read_mcp_resource".into(),
        description: "Read a resource exposed by a configured MCP server.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "resource": { "type": "string" },
                "uri": { "type": "string" }
            },
            "required": ["server", "resource"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    let server = input.get("server").and_then(|v| v.as_str()).unwrap_or("");
    let resource = input
        .get("resource")
        .or_else(|| input.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    json!({
        "ok": false,
        "server": server,
        "resource": resource,
        "message": "read_mcp_resource requires AppHandle-aware execution and should be routed via execute_tool_with_app."
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
    let uri = input
        .get("resource")
        .or_else(|| input.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    if server_name.is_empty() || uri.is_empty() {
        return json!({
            "ok": false,
            "error": "read_mcp_resource requires non-empty 'server' and 'resource'/'uri'"
        })
        .to_string();
    }

    match crate::command::mcp::read_mcp_resource(app.clone(), server_name, uri).await {
        Ok(v) => v.to_string(),
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    }
}
