pub use crate::llm::services::rag::types::{
    RagDocumentContent, RagDocumentInput, RagDocumentMeta, RagRejectedItem, RagSearchHit, RagStats,
    RagUpsertResult,
};

use tauri::AppHandle;

use crate::llm::utils::error_event::report_backend_result;

pub async fn rag_search_documents(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<RagSearchHit>, String> {
    crate::llm::services::rag::search_documents(app, query, limit).await
}

#[tauri::command]
pub async fn rag_read_document(
    app: AppHandle,
    document_id: String,
) -> Result<Option<RagDocumentContent>, String> {
    let result =
        crate::llm::services::rag::read_document(app.clone(), document_id).await;
    report_backend_result(&app, "command.rag.rag_read_document", result, None)
}

#[tauri::command]
pub async fn rag_get_stats(app: AppHandle) -> Result<RagStats, String> {
    let result = crate::llm::services::rag::get_stats(app.clone()).await;
    report_backend_result(&app, "command.rag.rag_get_stats", result, None)
}

#[tauri::command]
pub async fn rag_list_documents(app: AppHandle) -> Result<Vec<RagDocumentMeta>, String> {
    let result = crate::llm::services::rag::list_documents(app.clone()).await;
    report_backend_result(&app, "command.rag.rag_list_documents", result, None)
}

#[tauri::command]
pub async fn rag_upsert_documents(
    app: AppHandle,
    documents: Vec<RagDocumentInput>,
) -> Result<RagUpsertResult, String> {
    let result = crate::llm::services::rag::upsert_documents(app.clone(), documents).await;
    report_backend_result(&app, "command.rag.rag_upsert_documents", result, None)
}

#[tauri::command]
pub async fn rag_remove_document(app: AppHandle, document_id: String) -> Result<bool, String> {
    let result = crate::llm::services::rag::remove_document(app.clone(), document_id).await;
    report_backend_result(&app, "command.rag.rag_remove_document", result, None)
}

#[tauri::command]
pub async fn rag_clear_documents(app: AppHandle) -> Result<(), String> {
    let result = crate::llm::services::rag::clear_documents(app.clone()).await;
    report_backend_result(&app, "command.rag.rag_clear_documents", result, None)
}
