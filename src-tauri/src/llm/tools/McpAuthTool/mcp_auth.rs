use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把 MCP 管理操作的 async 逻辑包装成统一 future。
// `conversation_id` 只在 `probe_tool` 分支里继续往下传，用于嵌套权限确认。
fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

// 返回 mcp_auth 的注册信息。
// 这是写类工具，因为 enable/disable/reload/probe 都可能改变运行状态或触发远端调用。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回模型可见的 mcp_auth 元数据。
// `action` 决定本次是查状态、重载、切换开关、列工具还是探测调用。
pub fn tool() -> Tool {
    Tool {
        name: "mcp_auth".into(),
        description: "Gateway for MCP discovery and invocation: inspect connected servers, list a server's tools, and call a specific MCP tool on demand.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "reload_all", "enable", "disable", "list_tools", "probe_tool", "call_tool"]
                },
                "server": { "type": "string" },
                "tool": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["action"]
        }),
    }
}

// 执行 MCP 管理动作。
// `action` 决定分支；`server_name`、`tool_name`、`arguments` 只在对应分支中生效。
async fn execute_with_app(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    // action: 统一转成小写后的操作类型，避免大小写差异导致匹配失败。
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "status" => match crate::command::mcp::get_mcp_server_statuses(app.clone()).await {
            Ok(statuses) => Ok(ToolOutcome::json(json!({
                "ok": true,
                "action": "status",
                "servers": statuses
            }))),
            Err(e) => Err(ToolFailure::mcp(e)),
        },
        "reload_all" => {
            if let Err(e) = crate::command::mcp::reload_all_mcp_servers(app.clone()).await {
                return Err(ToolFailure::mcp(e));
            }
            match crate::command::mcp::get_mcp_server_statuses(app.clone()).await {
                Ok(statuses) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "reload_all",
                    "servers": statuses
                }))),
                Err(e) => Err(ToolFailure::mcp(e)),
            }
        }
        "enable" | "disable" => {
            let Some(server_name) = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Err(ToolFailure::invalid_input(
                    "mcp_auth action requires non-empty 'server'",
                ));
            };

            // enabled: true 表示启用 server，false 表示禁用 server。
            let enabled = action == "enable";
            match crate::command::mcp::set_mcp_server_enabled(
                app.clone(),
                server_name.to_string(),
                enabled,
            )
            .await
            {
                Ok(()) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": action,
                    "server": server_name,
                    "enabled": enabled
                }))),
                Err(e) => Err(ToolFailure::mcp(e)),
            }
        }
        "list_tools" => {
            let Some(server_name) = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Err(ToolFailure::invalid_input(
                    "mcp_auth list_tools requires non-empty 'server'",
                ));
            };

            match crate::command::mcp::list_mcp_tools(app.clone(), server_name.to_string()).await {
                Ok(tools) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "list_tools",
                    "server": server_name,
                    "tools": tools
                }))),
                Err(e) => Err(ToolFailure::mcp(e)),
            }
        }
        "probe_tool" | "call_tool" => {
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
            // arguments: 传给目标 MCP 工具的原始参数对象；缺失时默认空对象。
            let arguments = input
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            if server_name.is_empty() || tool_name.is_empty() {
                return Err(ToolFailure::invalid_input(
                    "mcp_auth probe_tool requires non-empty 'server' and 'tool'",
                ));
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
        _ => Err(ToolFailure::invalid_input(
            "mcp_auth action must be one of: status, reload_all, enable, disable, list_tools, probe_tool, call_tool",
        )),
    }
}
