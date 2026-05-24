use crate::llm::services::lsp::{
    LspDiagnosticsResponse, LspHoverResponse, LspRequestResponse, LspStatusResponse,
    LspSymbolsResponse,
};
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

#[tauri::command]
pub async fn lsp_definition(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
    line: u64,
    character: u64,
) -> Result<LspRequestResponse, String> {
    let app_handle = app.clone();
    report_backend_result(
        &app_handle,
        "command.lsp.definition",
        crate::llm::services::lsp::definition(
            &app,
            conversation_id.as_deref(),
            path,
            line,
            character,
        )
        .await,
        None,
    )
}

#[tauri::command]
pub async fn lsp_references(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
    line: u64,
    character: u64,
    include_declaration: Option<bool>,
) -> Result<LspRequestResponse, String> {
    let app_handle = app.clone();
    report_backend_result(
        &app_handle,
        "command.lsp.references",
        crate::llm::services::lsp::references(
            &app,
            conversation_id.as_deref(),
            path,
            line,
            character,
            include_declaration.unwrap_or(true),
        )
        .await,
        None,
    )
}

#[tauri::command]
pub async fn lsp_symbols(
    app: AppHandle,
    conversation_id: Option<String>,
    path: Option<String>,
    query: Option<String>,
) -> Result<LspSymbolsResponse, String> {
    let app_handle = app.clone();
    report_backend_result(
        &app_handle,
        "command.lsp.symbols",
        crate::llm::services::lsp::symbols(&app, conversation_id.as_deref(), path, query).await,
        None,
    )
}

#[tauri::command]
pub async fn lsp_hover(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
    line: u64,
    character: u64,
) -> Result<LspHoverResponse, String> {
    let app_handle = app.clone();
    report_backend_result(
        &app_handle,
        "command.lsp.hover",
        crate::llm::services::lsp::hover(&app, conversation_id.as_deref(), path, line, character)
            .await,
        None,
    )
}
