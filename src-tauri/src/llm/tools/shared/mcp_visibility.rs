use crate::llm::tools::ToolFailure;
use tauri::AppHandle;

/// 校验 MCP server 对当前会话可见。
///
/// 挂载智能体时按其 `enabled_mcp_servers` 引用清单过滤，未挂载时全部可见。
/// MCP 三个工具（mcp_auth / list_mcp_resources / read_mcp_resource）都必须走这里：
/// 只有 mcp_auth 做校验的话，被智能体禁用的 server 仍能从 resources 两个工具访问到。
pub(crate) async fn ensure_server_visible(
    app: &AppHandle,
    conversation_id: Option<&str>,
    server_name: &str,
) -> Result<(), ToolFailure> {
    let statuses = crate::command::mcp::get_mcp_server_statuses(app.clone())
        .await
        .map_err(ToolFailure::mcp)?;

    // 措辞与后端 services/mcp 的 "not found" 对齐，避免出现同一个问题两种说法。
    if !statuses.iter().any(|s| s.name == server_name) {
        return Err(ToolFailure::mcp(format!(
            "MCP server '{}' not found",
            server_name
        )));
    }

    if let Some(bundle) = crate::llm::services::agent_bundles::active_bundle(app, conversation_id) {
        if !bundle.is_mcp_server_enabled(server_name) {
            return Err(ToolFailure::mcp(format!(
                "MCP server '{}' is not enabled in this conversation",
                server_name
            )));
        }
    }

    Ok(())
}
