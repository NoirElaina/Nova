//! 桌面薄壳 IPC：转发至 nova daemon（方法名 usage.* 与内核分发器一一对应）。

pub use nova_protocol::misc::{
    ConversationUsageSummary, TokenUsageRecord, UsageStats,
};

#[tauri::command]
pub async fn get_usage_stats() -> Result<UsageStats, String> {
    crate::daemon::invoke("get_usage_stats", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn list_token_usage(limit: Option<i64>) -> Result<Vec<TokenUsageRecord>, String> {
    crate::daemon::invoke("list_token_usage", serde_json::json!({"limit": limit})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn get_conversation_usage(conversation_id: String) -> Result<ConversationUsageSummary, String> {
    crate::daemon::invoke("get_conversation_usage", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
