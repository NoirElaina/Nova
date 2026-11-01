//! 桌面薄壳 IPC：转发至 nova daemon（方法名 permission_rules.* 与内核分发器一一对应）。

use nova_protocol::misc::PermissionRule;

#[tauri::command]
pub async fn list_permission_rules() -> Result<Vec<PermissionRule>, String> {
    crate::daemon::invoke("list_permission_rules", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn delete_permission_rule(signature: String) -> Result<bool, String> {
    crate::daemon::invoke("delete_permission_rule", serde_json::json!({"signature": signature})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn submit_permission_decision(conversation_id: Option<String>, request_id: String, action: String) -> Result<bool, String> {
    crate::daemon::invoke("submit_permission_decision", serde_json::json!({"conversationId": conversation_id, "requestId": request_id, "action": action})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
