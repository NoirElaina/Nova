//! 持久化权限规则与审批裁决管理命令。

use tauri::AppHandle;

use crate::agent::permissions::rules::{self, PermissionRule};

#[tauri::command]
pub fn list_permission_rules(app: AppHandle) -> Result<Vec<PermissionRule>, String> {
    Ok(rules::load_rules(&app))
}

#[tauri::command]
pub fn delete_permission_rule(app: AppHandle, signature: String) -> Result<bool, String> {
    rules::remove_rule(&app, &signature)
}

#[tauri::command]
pub fn submit_permission_decision(
    app: AppHandle,
    conversation_id: Option<String>,
    request_id: String,
    action: String,
) -> Result<bool, String> {
    let action_enum = crate::agent::permissions::parse_permission_action_name(&action)
        .ok_or_else(|| format!("Unknown permission action: {}", action))?;
    crate::agent::permissions::submit_permission_decision(
        &app,
        conversation_id.as_deref(),
        &request_id,
        action_enum,
    )
}
