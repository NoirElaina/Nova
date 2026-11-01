//! 桌面薄壳 IPC：转发至 nova daemon（方法名 agent_config.* 与内核分发器一一对应）。

use nova_protocol::misc::AgentBundle;
use nova_protocol::misc::*;
pub use nova_protocol::misc::ConfigurableTool;

#[tauri::command]
pub async fn list_agent_bundles() -> Result<Vec<AgentBundle>, String> {
    crate::daemon::invoke("list_agent_bundles", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn create_agent_bundle(name: String) -> Result<AgentBundle, String> {
    crate::daemon::invoke("create_agent_bundle", serde_json::json!({"name": name})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn load_agent_bundle(bundle_id: String) -> Result<AgentBundle, String> {
    crate::daemon::invoke("load_agent_bundle", serde_json::json!({"bundleId": bundle_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn save_agent_bundle(bundle: AgentBundle) -> Result<AgentBundle, String> {
    crate::daemon::invoke("save_agent_bundle", serde_json::json!({"bundle": bundle})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn delete_agent_bundle(bundle_id: String) -> Result<(), String> {
    crate::daemon::invoke("delete_agent_bundle", serde_json::json!({"bundleId": bundle_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_conversation_agent(conversation_id: String) -> Result<Option<AgentBundle>, String> {
    crate::daemon::invoke("get_conversation_agent", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn set_conversation_agent(conversation_id: String, bundle_id: Option<String>) -> Result<Option<AgentBundle>, String> {
    crate::daemon::invoke("set_conversation_agent", serde_json::json!({"conversationId": conversation_id, "bundleId": bundle_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_configurable_tools() -> Result<Vec<ConfigurableTool>, String> {
    crate::daemon::invoke("list_configurable_tools", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_agent_private_skills(bundle_id: String) -> Result<Vec<nova_protocol::misc::SkillSummary>, String> {
    crate::daemon::invoke("list_agent_private_skills", serde_json::json!({"bundleId": bundle_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_agent_files(bundle_id: String) -> Result<Vec<AgentFileEntry>, String> {
    crate::daemon::invoke("list_agent_files", serde_json::json!({"bundleId": bundle_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn import_agent_file(bundle_id: String, src_path: String) -> Result<AgentFileEntry, String> {
    crate::daemon::invoke("import_agent_file", serde_json::json!({"bundleId": bundle_id, "srcPath": src_path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn delete_agent_file(bundle_id: String, name: String) -> Result<(), String> {
    crate::daemon::invoke("delete_agent_file", serde_json::json!({"bundleId": bundle_id, "name": name})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn reveal_agent_dir(bundle_id: String) -> Result<(), String> {
    crate::daemon::invoke("reveal_agent_dir", serde_json::json!({"bundleId": bundle_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
