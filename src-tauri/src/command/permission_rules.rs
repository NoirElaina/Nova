//! 持久化权限规则管理命令（设置页查看/删除"始终允许/拒绝"记录）。

use tauri::AppHandle;

use crate::llm::utils::permissions::rules::{self, PermissionRule};

#[tauri::command]
pub fn list_permission_rules(app: AppHandle) -> Result<Vec<PermissionRule>, String> {
    Ok(rules::load_rules(&app))
}

#[tauri::command]
pub fn delete_permission_rule(app: AppHandle, signature: String) -> Result<bool, String> {
    rules::remove_rule(&app, &signature)
}
