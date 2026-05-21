use crate::llm::services::browser_sessions::{self, BrowserAutomationResult};
use serde::Deserialize;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
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

const WEBVIEW_CALLBACK_TIMEOUT: Duration = Duration::from_secs(8);

async fn wait_webview_callback(
    rx: Receiver<Result<String, String>>,
    timeout_message: &'static str,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || match rx.recv_timeout(WEBVIEW_CALLBACK_TIMEOUT) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(timeout_message.to_string()),
        Err(RecvTimeoutError::Disconnected) => {
            Err("browser webview callback channel closed before returning a result".to_string())
        }
    })
    .await
    .map_err(|error| format!("browser webview callback wait failed: {error}"))?
}

#[tauri::command]
pub async fn browser_eval_webview_script_result(
    app: AppHandle,
    label: String,
    script: String,
) -> Result<String, String> {
    let webview = browser_webview(&app, &label)?;
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

    wait_webview_callback(rx, "browser script result timed out").await
}

#[tauri::command]
pub async fn browser_call_devtools_protocol_method(
    app: AppHandle,
    label: String,
    method: String,
    params_json: String,
) -> Result<String, String> {
    let webview = browser_webview(&app, &label)?;
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

    wait_webview_callback(rx, "browser devtools protocol call timed out").await
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
