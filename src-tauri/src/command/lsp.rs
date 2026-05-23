use crate::llm::services::lsp::{LspDiagnosticsResponse, LspStatusResponse};
use crate::llm::utils::error_event::report_backend_result;
use tauri::AppHandle;

#[tauri::command]
pub async fn lsp_status(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<LspStatusResponse, String> {
    let app_handle = app.clone();
    report_backend_result(
        &app_handle,
        "command.lsp.status",
        crate::llm::services::lsp::status(&app, conversation_id.as_deref()).await,
        None,
    )
}

#[tauri::command]
pub async fn lsp_diagnostics(
    app: AppHandle,
    conversation_id: Option<String>,
    path: Option<String>,
) -> Result<LspDiagnosticsResponse, String> {
    let app_handle = app.clone();
    report_backend_result(
        &app_handle,
        "command.lsp.diagnostics",
        crate::llm::services::lsp::diagnostics(&app, conversation_id.as_deref(), path).await,
        None,
    )
}
