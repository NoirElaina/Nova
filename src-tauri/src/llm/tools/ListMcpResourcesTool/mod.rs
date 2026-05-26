use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把读取 MCP 资源列表的 async 逻辑包装成统一 future。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 list_mcp_resources 的注册信息。
// 这是只读操作，只会向 MCP 查询资源目录。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回模型可见的 list_mcp_resources 元数据。
// `server` 指定要查询哪一个 MCP server。
pub fn tool() -> Tool {
    Tool {
        name: "list_mcp_resources".into(),
        description: "List resources exposed by a configured MCP server.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" }
            },
            "required": ["server"]
        }),
    }
}

// 调用后端 MCP 命令列出指定 server 的资源。
// `server_name` 是去掉空白后的服务器名，不能为空。
async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let server_name = input
        .get("server")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    if server_name.is_empty() {
        return Err(ToolFailure::invalid_input(
            "list_mcp_resources requires non-empty 'server'",
        ));
    }

    match crate::command::mcp::list_mcp_resources(app.clone(), server_name).await {
        Ok(v) => Ok(ToolOutcome::json(json!({ "ok": true, "resources": v }))),
        Err(e) => Err(ToolFailure::mcp(e)),
    }
}
