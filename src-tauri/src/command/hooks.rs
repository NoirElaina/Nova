//! 桌面薄壳 IPC：转发至 nova daemon（方法名 hooks.* 与内核分发器一一对应）。



#[tauri::command]
pub async fn get_hooks_toml() -> Result<String, String> {
    crate::daemon::invoke("get_hooks_toml", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_hooks_structured() -> Result<serde_json::Value, String> {
    crate::daemon::invoke("get_hooks_structured", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn save_hooks_toml(content: String) -> Result<usize, String> {
    crate::daemon::invoke("save_hooks_toml", serde_json::json!({"content": content})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
