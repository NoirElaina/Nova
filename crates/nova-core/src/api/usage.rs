pub use crate::provider::token_usage_log::{
    ConversationUsageSummary, TokenUsageRecord, UsageStats,
};


use crate::agent::utils::error_event::report_backend_result;

pub async fn get_usage_stats(core: &crate::core::Core) -> Result<UsageStats, String> {
    let result = crate::provider::token_usage_log::get_usage_stats(&core).await;
    report_backend_result(&core, "command.usage.get_usage_stats", result, None)
}

pub async fn list_token_usage(
    core: &crate::core::Core,
    limit: Option<i64>,
) -> Result<Vec<TokenUsageRecord>, String> {
    let result = crate::provider::token_usage_log::list_token_usage(&core, limit).await;
    report_backend_result(&core, "command.usage.list_token_usage", result, None)
}

pub async fn get_conversation_usage(
    core: &crate::core::Core,
    conversation_id: String,
) -> Result<ConversationUsageSummary, String> {
    let result =
        crate::provider::token_usage_log::get_conversation_usage(&core, &conversation_id).await;
    report_backend_result(&core, "command.usage.get_conversation_usage", result, None)
}
