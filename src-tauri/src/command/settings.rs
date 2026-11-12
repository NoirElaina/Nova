//! 桌面薄壳 IPC：转发至 nova daemon（方法名 settings.* 与内核分发器一一对应）。

pub use nova_protocol::settings::*;

#[tauri::command]
pub async fn get_settings() -> Result<AppSettings, String> {
    crate::daemon::invoke("get_settings", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn save_settings(settings: AppSettings) -> Result<(), String> {
    crate::daemon::invoke("save_settings", serde_json::json!({"settings": settings})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_model_window_tokens(model: String) -> u32 {
    match crate::daemon::invoke("get_model_window_tokens", serde_json::json!({"model": model})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(_) => Default::default(),
    }
}

#[tauri::command]
pub async fn estimate_text_tokens(text: String) -> u32 {
    match crate::daemon::invoke("estimate_text_tokens", serde_json::json!({"text": text})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(_) => Default::default(),
    }
}

#[tauri::command]
pub async fn get_available_terminals() -> Vec<nova_protocol::misc::TerminalInfo> {
    match crate::daemon::invoke("get_available_terminals", serde_json::json!({})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(_) => Default::default(),
    }
}

#[tauri::command]
pub async fn refresh_model_catalog() -> Result<nova_protocol::misc::ModelCatalogStats, String> {
    crate::daemon::invoke("refresh_model_catalog", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_model_catalog_stats() -> nova_protocol::misc::ModelCatalogStats {
    match crate::daemon::invoke("get_model_catalog_stats", serde_json::json!({})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(_) => Default::default(),
    }
}

#[tauri::command]
pub async fn get_model_catalog(search: Option<String>, modality: Option<String>) -> Vec<nova_protocol::misc::ModelCatalogSummary> {
    match crate::daemon::invoke("get_model_catalog", serde_json::json!({"search": search, "modality": modality})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(_) => Default::default(),
    }
}

#[tauri::command]
pub async fn get_sandbox_status() -> nova_protocol::misc::SandboxStatus {
    match crate::daemon::invoke("get_sandbox_status", serde_json::json!({})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(_) => Default::default(),
    }
}

#[tauri::command]
pub async fn export_diagnostic_bundle(target_path: Option<String>) -> Result<String, String> {
    let value = crate::daemon::invoke(
        "export_diagnostic_bundle",
        serde_json::json!({"targetPath": target_path}),
    )
    .await?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}
