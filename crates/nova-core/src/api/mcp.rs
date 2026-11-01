use crate::agent::utils::error_event::report_backend_result;
use serde_json::Value;

// 复用 services 层的 MCP 对外类型。
pub use crate::agent::capabilities::mcp::{
    McpResourceInfo, McpServerConfig, McpServerEntry, McpServerStatus, McpToolInfo,
};

pub async fn add_mcp_server(
    core: &crate::core::Core,
    name: String,
    config: McpServerConfig,
    source_agent: Option<String>,
) -> Result<(), String> {
    // 将 tauri 命令直接转发到 MCP service；source_agent 用于标注来源智能体。
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::add_mcp_server(core.clone(), name, config, source_agent).await;
    report_backend_result(&app_handle, "command.mcp.add_mcp_server", result, None)
}

pub async fn get_mcp_server(core: &crate::core::Core, name: String) -> Result<McpServerEntry, String> {
    // 读取指定 MCP server 的完整配置。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::get_mcp_server(core.clone(), name).await;
    report_backend_result(&app_handle, "command.mcp.get_mcp_server", result, None)
}

pub async fn update_mcp_server(
    core: &crate::core::Core,
    old_name: String,
    new_name: String,
    config: McpServerConfig,
) -> Result<(), String> {
    // 更新指定 MCP server，必要时允许改名并重连。
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::update_mcp_server(core.clone(), old_name, new_name, config).await;
    report_backend_result(&app_handle, "command.mcp.update_mcp_server", result, None)
}

pub async fn remove_mcp_server(core: &crate::core::Core, name: String) -> Result<(), String> {
    // 删除指定 MCP server 配置与运行态。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::remove_mcp_server(core.clone(), name).await;
    report_backend_result(&app_handle, "command.mcp.remove_mcp_server", result, None)
}

pub async fn get_mcp_server_statuses(core: &crate::core::Core) -> Result<Vec<McpServerStatus>, String> {
    // 获取所有 MCP server 当前状态。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::get_mcp_server_statuses(core.clone()).await;
    report_backend_result(
        &app_handle,
        "command.mcp.get_mcp_server_statuses",
        result,
        None,
    )
}

pub async fn reload_all_mcp_servers(core: &crate::core::Core) -> Result<(), String> {
    // 触发所有 MCP server 重载。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::reload_all_mcp_servers(core.clone()).await;
    report_backend_result(
        &app_handle,
        "command.mcp.reload_all_mcp_servers",
        result,
        None,
    )
}

pub async fn set_mcp_server_enabled(
    core: &crate::core::Core,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    // 启用或禁用指定 MCP server。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::set_mcp_server_enabled(core.clone(), name, enabled).await;
    report_backend_result(
        &app_handle,
        "command.mcp.set_mcp_server_enabled",
        result,
        None,
    )
}

pub async fn list_mcp_tools(
    core: &crate::core::Core,
    server_name: String,
) -> Result<Vec<McpToolInfo>, String> {
    // 列出指定 server 的工具。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::list_mcp_tools(core.clone(), server_name).await;
    report_backend_result(&app_handle, "command.mcp.list_mcp_tools", result, None)
}

pub async fn list_mcp_resources(
    core: &crate::core::Core,
    server_name: String,
) -> Result<Vec<McpResourceInfo>, String> {
    // 列出指定 server 的资源。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::list_mcp_resources(core.clone(), server_name).await;
    report_backend_result(&app_handle, "command.mcp.list_mcp_resources", result, None)
}

pub async fn read_mcp_resource(
    core: &crate::core::Core,
    server_name: String,
    uri: String,
) -> Result<Value, String> {
    // 读取指定资源 URI 的内容。
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::read_mcp_resource(core.clone(), server_name, uri).await;
    report_backend_result(&app_handle, "command.mcp.read_mcp_resource", result, None)
}

pub async fn call_mcp_tool(
    core: &crate::core::Core,
    server_name: String,
    tool_name: String,
    arguments: Value,
) -> Result<Value, String> {
    // 调用指定 MCP 工具并返回原始结果 JSON。
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::call_mcp_tool(core.clone(), server_name, tool_name, arguments).await;
    report_backend_result(&app_handle, "command.mcp.call_mcp_tool", result, None)
}

pub async fn warmup_runtime(core: &crate::core::Core) -> Result<(), String> {
    // 应用启动时预热 MCP runtime（非 tauri command）。
    crate::agent::capabilities::mcp::warmup_runtime(core.clone()).await
}
