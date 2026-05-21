use tauri::{AppHandle, Manager, Url};

fn browser_webview(app: &AppHandle, label: &str) -> Result<tauri::Webview, String> {
    app.get_webview(label)
        .ok_or_else(|| format!("browser webview not found: {label}"))
}

#[tauri::command]
pub fn browser_navigate_webview(
    app: AppHandle,
    label: String,
    url: String,
) -> Result<(), String> {
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
