use crate::services::user_terminal::{self, UserTerminalInfo};
use tauri::AppHandle;

#[tauri::command]
pub async fn user_terminal_start(
    app: AppHandle,
    conversation_id: Option<String>,
    terminal_id: Option<String>,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<UserTerminalInfo, String> {
    let workspace_root = crate::daemon::invoke(
        "get_workspace_root",
        serde_json::json!({ "conversationId": conversation_id }),
    )
    .await
    .and_then(|v| serde_json::from_value::<String>(v).map_err(|e| e.to_string()))?;
    let active_terminal = crate::daemon::invoke("get_active_terminal", serde_json::json!({}))
        .await
        .and_then(|v| {
            serde_json::from_value::<nova_protocol::misc::TerminalInfo>(v)
                .map_err(|e| e.to_string())
        })?;
    user_terminal::start_session(
        app,
        conversation_id.as_deref(),
        terminal_id.as_deref(),
        std::path::Path::new(&workspace_root),
        &active_terminal,
        rows,
        cols,
    )
}

#[tauri::command]
pub fn user_terminal_write(
    conversation_id: Option<String>,
    terminal_id: Option<String>,
    data: String,
) -> Result<(), String> {
    user_terminal::write_session(conversation_id.as_deref(), terminal_id.as_deref(), data)
}

#[tauri::command]
pub fn user_terminal_resize(
    conversation_id: Option<String>,
    terminal_id: Option<String>,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<(), String> {
    user_terminal::resize_session(conversation_id.as_deref(), terminal_id.as_deref(), rows, cols)
}

#[tauri::command]
pub fn user_terminal_stop(
    conversation_id: Option<String>,
    terminal_id: Option<String>,
) -> Result<(), String> {
    user_terminal::stop_terminal(conversation_id.as_deref(), terminal_id.as_deref())
}
