use tauri::AppHandle;
use tauri::Manager;

#[tauri::command]
pub fn get_tool_path(app: AppHandle, name: String) -> Result<String, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e: tauri::Error| e.to_string())?;

    let tool_path = match name.as_str() {
        "rg" => resource_dir.join("bin").join(if cfg!(target_os = "windows") {
            "rg.exe"
        } else {
            "rg"
        }),
        _ => return Err(format!("Unknown tool: {}", name)),
    };

    if tool_path.exists() {
        Ok(tool_path.to_string_lossy().to_string())
    } else {
        Err(format!("Tool not found: {}", tool_path.display()))
    }
}
