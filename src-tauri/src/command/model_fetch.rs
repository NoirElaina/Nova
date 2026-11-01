//! 桌面薄壳 IPC：转发至 nova daemon（方法名 model_fetch.* 与内核分发器一一对应）。

use nova_protocol::misc::*;

#[tauri::command]
pub async fn fetch_available_models(base_url: String, api_key: String, is_full_url: Option<bool>, models_url_override: Option<String>, api_format: Option<String>) -> Result<Vec<FetchedModel>, String> {
    crate::daemon::invoke("fetch_available_models", serde_json::json!({"baseUrl": base_url, "apiKey": api_key, "isFullUrl": is_full_url, "modelsUrlOverride": models_url_override, "apiFormat": api_format})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
