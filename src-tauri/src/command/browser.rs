use crate::llm::services::browser_sessions::{self, BrowserAutomationResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tauri::{AppHandle, Manager, Url};

fn browser_window_webview(app: &AppHandle, label: &str) -> Result<tauri::Webview, String> {
    app.get_webview(label)
        .ok_or_else(|| format!("browser window not found: {label}"))
}

#[tauri::command]
pub fn browser_navigate_window(app: AppHandle, label: String, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|error| error.to_string())?;
    browser_window_webview(&app, &label)?
        .navigate(parsed)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn browser_reload_window(app: AppHandle, label: String) -> Result<(), String> {
    browser_window_webview(&app, &label)?
        .reload()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn browser_eval_window_script(
    app: AppHandle,
    label: String,
    script: String,
) -> Result<(), String> {
    browser_window_webview(&app, &label)?
        .eval(script)
        .map_err(|error| error.to_string())
}

const BROWSER_WINDOW_CALLBACK_TIMEOUT: Duration = Duration::from_secs(8);

async fn wait_browser_window_callback(
    rx: Receiver<Result<String, String>>,
    timeout_message: &'static str,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        match rx.recv_timeout(BROWSER_WINDOW_CALLBACK_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(timeout_message.to_string()),
            Err(RecvTimeoutError::Disconnected) => {
                Err("browser window callback channel closed before returning a result".to_string())
            }
        }
    })
    .await
    .map_err(|error| format!("browser window callback wait failed: {error}"))?
}

#[tauri::command]
pub async fn browser_eval_window_script_result(
    app: AppHandle,
    label: String,
    script: String,
) -> Result<String, String> {
    let webview = browser_window_webview(&app, &label)?;
    let (tx, rx) = mpsc::channel();

    webview
        .with_webview(move |platform_webview| {
            if let Err(error) = eval_platform_webview_script_result(platform_webview, script, tx) {
                // The WebView callback path reports async completion; only immediate setup
                // errors are sent here.
                tracing::warn!(error = %error, "failed to start browser script result evaluation");
            }
        })
        .map_err(|error| error.to_string())?;

    wait_browser_window_callback(rx, "browser script result timed out").await
}

#[tauri::command]
pub async fn browser_call_devtools_protocol_method(
    app: AppHandle,
    label: String,
    method: String,
    params_json: String,
) -> Result<String, String> {
    let webview = browser_window_webview(&app, &label)?;
    let (tx, rx) = mpsc::channel();

    webview
        .with_webview(move |platform_webview| {
            if let Err(error) =
                call_platform_devtools_protocol_method(platform_webview, method, params_json, tx)
            {
                tracing::warn!(error = %error, "failed to start browser devtools protocol call");
            }
        })
        .map_err(|error| error.to_string())?;

    wait_browser_window_callback(rx, "browser devtools protocol call timed out").await
}

#[cfg(windows)]
fn eval_platform_webview_script_result(
    platform_webview: tauri::webview::PlatformWebview,
    script: String,
    tx: Sender<Result<String, String>>,
) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, ExecuteScriptCompletedHandler};

    let controller = platform_webview.controller();
    let webview = unsafe { controller.CoreWebView2() }.map_err(|error| error.to_string())?;

    let handler = ExecuteScriptCompletedHandler::create(Box::new(move |error_code, result| {
        let payload = error_code
            .map(|_| result)
            .map_err(|error| error.to_string());
        let _ = tx.send(payload);
        Ok(())
    }));
    let js = CoTaskMemPWSTR::from(script.as_str());
    unsafe {
        webview
            .ExecuteScript(*js.as_ref().as_pcwstr(), &handler)
            .map_err(|error| error.to_string())
    }
}

#[cfg(windows)]
fn call_platform_devtools_protocol_method(
    platform_webview: tauri::webview::PlatformWebview,
    method: String,
    params_json: String,
    tx: Sender<Result<String, String>>,
) -> Result<(), String> {
    use webview2_com::{CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR};

    let controller = platform_webview.controller();
    let webview = unsafe { controller.CoreWebView2() }.map_err(|error| error.to_string())?;

    let handler =
        CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |error_code, result| {
            let payload = error_code
                .map(|_| result)
                .map_err(|error| error.to_string());
            let _ = tx.send(payload);
            Ok(())
        }));
    let method = CoTaskMemPWSTR::from(method.as_str());
    let params = CoTaskMemPWSTR::from(params_json.as_str());
    unsafe {
        webview
            .CallDevToolsProtocolMethod(
                *method.as_ref().as_pcwstr(),
                *params.as_ref().as_pcwstr(),
                &handler,
            )
            .map_err(|error| error.to_string())
    }
}

#[cfg(not(windows))]
fn eval_platform_webview_script_result(
    _platform_webview: tauri::webview::PlatformWebview,
    _script: String,
    tx: Sender<Result<String, String>>,
) -> Result<(), String> {
    let _ = tx.send(Err(
        "browser script result is only implemented for Windows WebView2".to_string(),
    ));
    Ok(())
}

#[cfg(not(windows))]
fn call_platform_devtools_protocol_method(
    _platform_webview: tauri::webview::PlatformWebview,
    _method: String,
    _params_json: String,
    tx: Sender<Result<String, String>>,
) -> Result<(), String> {
    let _ = tx.send(Err(
        "browser devtools protocol is only implemented for Windows WebView2".to_string(),
    ));
    Ok(())
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabState {
    pub address_input: String,
    pub current_url: String,
    pub history: Vec<String>,
    pub history_index: i64,
    pub zoom_percent: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabStateInput {
    pub address_input: String,
    pub current_url: String,
    pub history: Vec<String>,
    pub history_index: i64,
    pub zoom_percent: i64,
}

fn browser_state_root(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to resolve app_data_dir for browser state: {error}"))?;
    Ok(app_data_dir.join("browser-tabs"))
}

fn browser_state_file_name(conversation_id: Option<&str>) -> String {
    let raw = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__default__");
    let mut safe = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            safe.push(ch);
        } else {
            safe.push('_');
        }
    }
    let safe = safe.trim_matches('.');
    if safe.is_empty() {
        "__default__.json".to_string()
    } else {
        format!("{safe}.json")
    }
}

fn browser_state_path(app: &AppHandle, conversation_id: Option<&str>) -> Result<PathBuf, String> {
    Ok(browser_state_root(app)?.join(browser_state_file_name(conversation_id)))
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[tauri::command]
pub fn load_browser_tab_state(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Option<BrowserTabState>, String> {
    let path = browser_state_path(&app, conversation_id.as_deref())?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str::<BrowserTabState>(&text)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_browser_tab_state(
    app: AppHandle,
    conversation_id: Option<String>,
    state: BrowserTabStateInput,
) -> Result<(), String> {
    let root = browser_state_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let path = browser_state_path(&app, conversation_id.as_deref())?;
    let state = BrowserTabState {
        address_input: state.address_input,
        current_url: state.current_url,
        history: state.history,
        history_index: state.history_index,
        zoom_percent: state.zoom_percent,
        updated_at: now_unix_ms(),
    };
    let text = serde_json::to_string_pretty(&state).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn clear_browser_tab_state(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<(), String> {
    let path = browser_state_path(&app, conversation_id.as_deref())?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}
