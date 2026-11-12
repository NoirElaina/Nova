use crate::agent::utils::error_event::report_backend_result;
use crate::NovaCoreError;
use serde_json::Value;

// 复用 services 层的 MCP 对外类型。
pub use crate::agent::capabilities::mcp::{
    McpElicitationDecision, McpNegotiatedCapabilities, McpOAuthLoginResult, McpResourceInfo,
    McpServerConfig, McpServerEntry, McpServerStatus, McpToolInfo,
};

pub async fn add_mcp_server(
    core: &crate::core::Core,
    name: String,
    config: McpServerConfig,
    source_agent: Option<String>,
) -> Result<(), NovaCoreError> {
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::add_mcp_server(core.clone(), name, config, source_agent)
            .await;
    report_backend_result(&app_handle, "command.mcp.add_mcp_server", result, None)
}

pub async fn get_mcp_server(
    core: &crate::core::Core,
    name: String,
) -> Result<McpServerEntry, NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::get_mcp_server(core.clone(), name).await;
    report_backend_result(&app_handle, "command.mcp.get_mcp_server", result, None)
}

pub async fn update_mcp_server(
    core: &crate::core::Core,
    old_name: String,
    new_name: String,
    config: McpServerConfig,
) -> Result<(), NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::update_mcp_server(
        core.clone(),
        old_name,
        new_name,
        config,
    )
    .await;
    report_backend_result(&app_handle, "command.mcp.update_mcp_server", result, None)
}

pub async fn remove_mcp_server(
    core: &crate::core::Core,
    name: String,
) -> Result<(), NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::remove_mcp_server(core.clone(), name).await;
    report_backend_result(&app_handle, "command.mcp.remove_mcp_server", result, None)
}

pub async fn get_mcp_server_statuses(
    core: &crate::core::Core,
) -> Result<Vec<McpServerStatus>, NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::get_mcp_server_statuses(core.clone()).await;
    report_backend_result(
        &app_handle,
        "command.mcp.get_mcp_server_statuses",
        result,
        None,
    )
}

pub async fn reload_all_mcp_servers(core: &crate::core::Core) -> Result<(), NovaCoreError> {
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
) -> Result<(), NovaCoreError> {
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::set_mcp_server_enabled(core.clone(), name, enabled).await;
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
) -> Result<Vec<McpToolInfo>, NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::list_mcp_tools(core.clone(), server_name).await;
    report_backend_result(&app_handle, "command.mcp.list_mcp_tools", result, None)
}

pub async fn list_mcp_resources(
    core: &crate::core::Core,
    server_name: String,
) -> Result<Vec<McpResourceInfo>, NovaCoreError> {
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::list_mcp_resources(core.clone(), server_name).await;
    report_backend_result(&app_handle, "command.mcp.list_mcp_resources", result, None)
}

pub async fn read_mcp_resource(
    core: &crate::core::Core,
    server_name: String,
    uri: String,
) -> Result<Value, NovaCoreError> {
    let app_handle = core.clone();
    let result =
        crate::agent::capabilities::mcp::read_mcp_resource(core.clone(), server_name, uri).await;
    report_backend_result(&app_handle, "command.mcp.read_mcp_resource", result, None)
}

pub async fn call_mcp_tool(
    core: &crate::core::Core,
    server_name: String,
    tool_name: String,
    arguments: Value,
) -> Result<Value, NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::call_mcp_tool(
        core.clone(),
        server_name,
        tool_name,
        arguments,
    )
    .await;
    report_backend_result(&app_handle, "command.mcp.call_mcp_tool", result, None)
}

pub async fn mcp_oauth_login(
    core: &crate::core::Core,
    server_name: String,
    client_id: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<McpOAuthLoginResult, NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::mcp_oauth_login(
        core.clone(),
        server_name,
        client_id,
        scopes.unwrap_or_default(),
    )
    .await;
    report_backend_result(&app_handle, "command.mcp.mcp_oauth_login", result, None)
}

pub async fn mcp_oauth_logout(
    core: &crate::core::Core,
    server_name: String,
) -> Result<McpOAuthLoginResult, NovaCoreError> {
    let app_handle = core.clone();
    let result = crate::agent::capabilities::mcp::mcp_oauth_logout(core.clone(), server_name).await;
    report_backend_result(&app_handle, "command.mcp.mcp_oauth_logout", result, None)
}

pub fn resolve_mcp_elicitation(
    core: &crate::core::Core,
    request_id: String,
    decision: McpElicitationDecision,
    caller_client_id: Option<&str>,
) -> Result<bool, NovaCoreError> {
    let result = crate::agent::capabilities::mcp::resolve_mcp_elicitation(
        request_id,
        decision,
        caller_client_id,
    );
    report_backend_result(core, "command.mcp.resolve_mcp_elicitation", result, None)
}

pub async fn warmup_runtime(core: &crate::core::Core) -> Result<(), NovaCoreError> {
    crate::agent::capabilities::mcp::warmup_runtime(core.clone()).await
}
