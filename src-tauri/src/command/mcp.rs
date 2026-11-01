//! 桌面薄壳 IPC：转发至 nova daemon（方法名 mcp.* 与内核分发器一一对应）。

use serde_json::Value;
pub use nova_protocol::mcp::{
    McpResourceInfo, McpServerConfig, McpServerEntry, McpServerStatus, McpToolInfo,
};

#[tauri::command]
pub async fn add_mcp_server(name: String, config: McpServerConfig, source_agent: Option<String>) -> Result<(), String> {
    crate::daemon::invoke("add_mcp_server", serde_json::json!({"name": name, "config": config, "sourceAgent": source_agent})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_mcp_server(name: String) -> Result<McpServerEntry, String> {
    crate::daemon::invoke("get_mcp_server", serde_json::json!({"name": name})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn update_mcp_server(old_name: String, new_name: String, config: McpServerConfig) -> Result<(), String> {
    crate::daemon::invoke("update_mcp_server", serde_json::json!({"oldName": old_name, "newName": new_name, "config": config})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn remove_mcp_server(name: String) -> Result<(), String> {
    crate::daemon::invoke("remove_mcp_server", serde_json::json!({"name": name})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_mcp_server_statuses() -> Result<Vec<McpServerStatus>, String> {
    crate::daemon::invoke("get_mcp_server_statuses", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn reload_all_mcp_servers() -> Result<(), String> {
    crate::daemon::invoke("reload_all_mcp_servers", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn set_mcp_server_enabled(name: String, enabled: bool) -> Result<(), String> {
    crate::daemon::invoke("set_mcp_server_enabled", serde_json::json!({"name": name, "enabled": enabled})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_mcp_tools(server_name: String) -> Result<Vec<McpToolInfo>, String> {
    crate::daemon::invoke("list_mcp_tools", serde_json::json!({"serverName": server_name})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_mcp_resources(server_name: String) -> Result<Vec<McpResourceInfo>, String> {
    crate::daemon::invoke("list_mcp_resources", serde_json::json!({"serverName": server_name})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn read_mcp_resource(server_name: String, uri: String) -> Result<Value, String> {
    crate::daemon::invoke("read_mcp_resource", serde_json::json!({"serverName": server_name, "uri": uri})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn call_mcp_tool(server_name: String, tool_name: String, arguments: Value) -> Result<Value, String> {
    crate::daemon::invoke("call_mcp_tool", serde_json::json!({"serverName": server_name, "toolName": tool_name, "arguments": arguments})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
