pub use crate::llm::services::token_usage_log::{
    TokenUsageRecord, UsageStats,
};

use tauri::AppHandle;

use crate::llm::utils::error_event::report_backend_result;

#[tauri::command]
pub async fn get_usage_stats(app: AppHandle) -> Result<UsageStats, String> {
    let result = crate::llm::services::token_usage_log::get_usage_stats(&app).await;
    report_backend_result(&app, "command.usage.get_usage_stats", result, None)
}

#[tauri::command]
pub async fn list_token_usage(
    app: AppHandle,
    limit: Option<i64>,
) -> Result<Vec<TokenUsageRecord>, String> {
    let result = crate::llm::services::token_usage_log::list_token_usage(&app, limit).await;
    report_backend_result(&app, "command.usage.list_token_usage", result, None)
}
