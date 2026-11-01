//! 桌面薄壳 IPC：转发至 nova daemon（方法名 skill.* 与内核分发器一一对应）。

use nova_protocol::misc::SkillSummary;

#[tauri::command]
pub async fn list_skills() -> Result<Vec<SkillSummary>, String> {
    crate::daemon::invoke("list_skills", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn delete_skill(path: String) -> Result<(), String> {
    crate::daemon::invoke("delete_skill", serde_json::json!({"path": path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn read_skill_content(path: String) -> Result<String, String> {
    crate::daemon::invoke("read_skill_content", serde_json::json!({"path": path})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
