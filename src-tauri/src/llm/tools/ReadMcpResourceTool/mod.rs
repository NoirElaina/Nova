use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把读取 MCP 资源内容的 async 逻辑包装成统一 future。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 read_mcp_resource 的注册信息。
// 这是只读操作，只读取 MCP 暴露的资源内容。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回模型可见的 read_mcp_resource 元数据。
// 当前接口统一使用 `uri` 作为 MCP 资源标识。
pub fn tool() -> Tool {
    Tool {
        name: "read_mcp_resource".into(),
        description: "Read a resource exposed by a configured MCP server.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "uri": { "type": "string" }
            },
            "required": ["server", "uri"]
        }),
    }
}

// 从指定 MCP server 读取一个资源。
// `server_name` 标识服务器，`uri` 标识资源地址，二者都会先去掉首尾空白。
async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let server_name = input
        .get("server")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let uri = input
        .get("uri")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    if server_name.is_empty() || uri.is_empty() {
        return Err(ToolFailure::invalid_input(
            "read_mcp_resource requires non-empty 'server' and 'uri'",
        ));
    }

    match crate::command::mcp::read_mcp_resource(app.clone(), server_name, uri).await {
        Ok(v) => Ok(ToolOutcome::json(v)),
        Err(e) => Err(ToolFailure::mcp(e)),
    }
}
