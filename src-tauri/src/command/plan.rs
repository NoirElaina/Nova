//! 桌面薄壳 IPC：转发至 nova daemon（方法名 plan.* 与内核分发器一一对应）。

use nova_protocol::misc::*;



#[tauri::command]
pub async fn get_conversation_plan(conversation_id: Option<String>) -> Result<Option<ConversationPlan>, String> {
    crate::daemon::invoke("get_conversation_plan", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
