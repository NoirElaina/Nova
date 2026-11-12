use std::sync::{OnceLock, RwLock};
use crate::core::Core;

use crate::agent::tools::{ToolExecResult, ToolFailure, ToolOutcome};
use crate::provider::types::Tool;

static CACHED_MCP_TOOLS: OnceLock<RwLock<Vec<Tool>>> = OnceLock::new();

fn mcp_tools_cache() -> &'static RwLock<Vec<Tool>> {
    CACHED_MCP_TOOLS.get_or_init(|| RwLock::new(Vec::new()))
}

/// 同步读取已缓存的 MCP 工具定义列表，供三协议适配器及工具查找同步热路径读取。
pub fn cached_mcp_tools() -> Vec<Tool> {
    mcp_tools_cache()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// 更新内存中缓存的 MCP 工具定义列表。
pub fn update_cached_mcp_tools(tools: Vec<Tool>) {
    if let Ok(mut w) = mcp_tools_cache().write() {
        *w = tools;
    }
}

/// 根据完整工具名（mcp__<server>__<tool>）查找已缓存的定义。
pub fn find_cached_mcp_tool(name: &str) -> Option<Tool> {
    mcp_tools_cache()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|t| t.name == name)
        .cloned()
}

/// 增量刷新单个 MCP 服务在内存缓存中的工具列表（供 `notifications/tools/list_changed` 热更新）。
pub fn refresh_server_tools_cache(
    server_name: &str,
    server_tools: &[crate::agent::capabilities::mcp::McpToolInfo],
) {
    let prefix = format!("mcp__{}__", server_name.trim());
    if let Ok(mut w) = mcp_tools_cache().write() {
        w.retain(|tool| !tool.name.starts_with(&prefix));
        for t in server_tools {
            w.push(Tool {
                name: build_mcp_tool_name(server_name, &t.name),
                description: t.description.clone().unwrap_or_else(|| {
                    format!("MCP tool '{}' from server '{}'.", t.name, server_name)
                }),
                input_schema: t
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} })),
            });
        }
    }
}

/// 异步拉取已连接 MCP 服务器的全部工具列表，并原子更新同步内存缓存。
pub async fn refresh_mcp_tools_cache(core: &Core) -> Vec<Tool> {
    let tools = collect_mcp_tools(core).await;
    update_cached_mcp_tools(tools.clone());
    tools
}

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

pub fn dynamic_tool_read_only(_name: &str) -> Option<bool> {
    // MCP 外部工具一律不标记为只读，避免误判导致并发执行副作用工具
    None
}

pub(crate) async fn execute_dynamic_with_app(
    core: &Core,
    conversation_id: Option<&str>,
    name: &str,
    arguments: serde_json::Value,
) -> Option<ToolExecResult> {
    let (server_name, tool_name) = parse_mcp_tool_name(name)?;
    Some(
        match crate::agent::capabilities::mcp::call_mcp_tool_with_context(
            core.clone(),
            conversation_id.map(str::to_string),
            server_name,
            tool_name,
            arguments,
            None,
        )
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
    core: &Core,
) -> Vec<crate::agent::capabilities::mcp::McpServerStatus> {
    let statuses = match crate::agent::capabilities::mcp::get_mcp_server_statuses(core.clone()).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let has_enabled = statuses.iter().any(|s| s.enabled);
    let has_connected = statuses
        .iter()
        .any(|s| s.enabled && s.status == "connected");

    if has_enabled && !has_connected {
        // 后台触发重连，不阻塞当前查询路径。
        // 重连可能耗时 30s（MCP_CONNECT_TIMEOUT），若在此等待会导致用户消息卡住。
        // 重连完成后下一轮查询自然获取到已连接的服务器。
        let app_clone = core.clone();
        tokio::spawn(async move {
            let _ = crate::agent::capabilities::mcp::reload_all_mcp_servers(app_clone).await;
        });
    }

    // 只返回已连接的服务器，不等待重连结果。
    statuses
        .into_iter()
        .filter(|s| s.enabled && s.status == "connected")
        .collect()
}

// 查询已启用并已连接的 MCP 服务器，收集每个 server 的 tool 列表并转成本地 Tool 格式。
// 这将使模型可调用 "mcp__<server>__<tool>"。
pub async fn collect_mcp_tools(core: &Core) -> Vec<Tool> {
    let mut tools_vec = Vec::new();
    for status in connected_server_catalog(core).await {
        let listed =
            match crate::agent::capabilities::mcp::list_mcp_tools(core.clone(), status.name.clone()).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tool_name_roundtrip() {
        let name = build_mcp_tool_name("github", "create_issue");
        assert_eq!(name, "mcp__github__create_issue");

        let parsed = parse_mcp_tool_name(&name);
        assert_eq!(parsed, Some(("github".to_string(), "create_issue".to_string())));

        assert_eq!(parse_mcp_tool_name("regular_tool"), None);
        assert_eq!(parse_mcp_tool_name("mcp____tool"), None);
    }

    #[test]
    fn test_cached_mcp_tools_storage_and_lookup() {
        let sample = vec![
            Tool {
                name: "mcp__sqlite__query".into(),
                description: "Run SQL query".into(),
                input_schema: serde_json::json!({ "type": "object" }),
            },
            Tool {
                name: "mcp__fetch__get".into(),
                description: "HTTP GET request".into(),
                input_schema: serde_json::json!({ "type": "object" }),
            },
        ];

        update_cached_mcp_tools(sample);

        let cached = cached_mcp_tools();
        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0].name, "mcp__sqlite__query");

        let found = find_cached_mcp_tool("mcp__fetch__get");
        assert!(found.is_some());
        assert_eq!(found.unwrap().description, "HTTP GET request");

        assert!(find_cached_mcp_tool("mcp__nonexistent__tool").is_none());
    }
}
