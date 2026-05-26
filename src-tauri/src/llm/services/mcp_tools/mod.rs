use tauri::AppHandle;

use crate::llm::tools::{ToolExecResult, ToolFailure, ToolOutcome};
use crate::llm::types::Tool;

pub fn build_mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{}__{}", server.trim(), tool.trim())
}

// 解析 mcp 形式工具名："mcp__<server>__<tool>" -> (server, tool)
pub fn parse_mcp_tool_name(name: &str) -> Option<(String, String)> {
    let raw = name.strip_prefix("mcp__")?;
    let mut parts = raw.splitn(2, "__");
    let server = parts.next()?.trim();
    let tool = parts.next()?.trim();
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server.to_string(), tool.to_string()))
}

pub fn dynamic_tool_read_only(name: &str) -> Option<bool> {
    let (_server_name, tool_name) = parse_mcp_tool_name(name)?;
    let tool_lower = tool_name.to_ascii_lowercase();
    Some(
        ["read", "list", "search", "get", "fetch", "glob", "grep"]
            .iter()
            .any(|kw| tool_lower.contains(kw)),
    )
}

pub(crate) async fn execute_dynamic_with_app(
    app: &AppHandle,
    name: &str,
    arguments: serde_json::Value,
) -> Option<ToolExecResult> {
    let (server_name, tool_name) = parse_mcp_tool_name(name)?;
    Some(
        match crate::command::mcp::call_mcp_tool(app.clone(), server_name, tool_name, arguments)
            .await
        {
            Ok(v) if v.get("isError").and_then(|value| value.as_bool()) == Some(true) => {
                Err(ToolFailure::mcp(v.to_string()))
            }
            Ok(v) => Ok(ToolOutcome::json(v)),
            Err(e) => Err(ToolFailure::mcp(e)),
        },
    )
}

pub async fn connected_server_catalog(
    app: &AppHandle,
) -> Vec<crate::llm::services::mcp::McpServerStatus> {
    let mut statuses = match crate::llm::services::mcp::get_mcp_server_statuses(app.clone()).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let has_enabled = statuses.iter().any(|s| s.enabled);
    let has_connected = statuses
        .iter()
        .any(|s| s.enabled && s.status == "connected");
    if has_enabled && !has_connected {
        let _ = crate::llm::services::mcp::reload_all_mcp_servers(app.clone()).await;
        statuses = match crate::llm::services::mcp::get_mcp_server_statuses(app.clone()).await {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
    }

    statuses
        .into_iter()
        .filter(|s| s.enabled && s.status == "connected")
        .collect()
}

// 查询已启用并已连接的 MCP 服务器，收集每个 server 的 tool 列表并转成本地 Tool 格式。
// 这将使模型可调用 "mcp__<server>__<tool>"。
pub async fn collect_mcp_tools(app: &AppHandle) -> Vec<Tool> {
    let mut tools_vec = Vec::new();
    for status in connected_server_catalog(app).await {
        let listed =
            match crate::llm::services::mcp::list_mcp_tools(app.clone(), status.name.clone()).await
            {
                Ok(v) => v,
                Err(_) => continue,
            };

        for t in listed {
            tools_vec.push(Tool {
                name: build_mcp_tool_name(&status.name, &t.name),
                description: t.description.unwrap_or_else(|| {
                    format!("MCP tool '{}' from server '{}'.", t.name, status.name)
                }),
                input_schema: t
                    .input_schema
                    .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} })),
            });
        }
    }

    tools_vec
}
