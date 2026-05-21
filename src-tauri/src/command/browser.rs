use crate::llm::services::browser_sessions::{self, BrowserAutomationResult};
use serde::Deserialize;
use tauri::{AppHandle, Manager, Url};

fn browser_webview(app: &AppHandle, label: &str) -> Result<tauri::Webview, String> {
    app.get_webview(label)
        .ok_or_else(|| format!("browser webview not found: {label}"))
}

#[tauri::command]
pub fn browser_navigate_webview(app: AppHandle, label: String, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|error| error.to_string())?;
    browser_webview(&app, &label)?
        .navigate(parsed)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn browser_reload_webview(app: AppHandle, label: String) -> Result<(), String> {
    browser_webview(&app, &label)?
        .reload()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn browser_eval_webview_script(
    app: AppHandle,
    label: String,
    script: String,
) -> Result<(), String> {
    browser_webview(&app, &label)?
        .eval(script)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn register_browser_session(
    conversation_id: Option<String>,
    label: String,
    current_url: Option<String>,
) -> Result<(), String> {
    browser_sessions::register_session(conversation_id.as_deref(), label, current_url)
}

#[tauri::command]
pub fn unregister_browser_session(
    conversation_id: Option<String>,
    label: String,
) -> Result<(), String> {
    browser_sessions::unregister_session(conversation_id.as_deref(), &label)
}

#[tauri::command]
pub fn update_browser_session_url(
    conversation_id: Option<String>,
    label: String,
    current_url: Option<String>,
) -> Result<(), String> {
    browser_sessions::update_session_url(conversation_id.as_deref(), &label, current_url)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationResultPayload {
    request_id: String,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
}

#[tauri::command]
pub fn browser_automation_result(payload: BrowserAutomationResultPayload) -> Result<(), String> {
    browser_sessions::complete_command(BrowserAutomationResult {
        request_id: payload.request_id,
        ok: payload.ok,
        result: payload.result,
        error: payload.error,
    })
}
