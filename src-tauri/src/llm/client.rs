use tauri::AppHandle;
use crate::llm::utils::error_event::report_backend_result;


// 对外复用 types 的事件类型定义。
pub use crate::llm::types::ChatMessageEvent;


#[tauri::command]
pub async fn get_chat_turn_status(
    conversation_id: Option<String>,
) -> Result<Option<crate::llm::services::live_turns::LiveTurnStatus>, String> {
    Ok(crate::llm::services::live_turns::get_status(
        conversation_id.as_deref(),
    ))
}

#[tauri::command]
pub async fn ack_chat_turn_status(conversation_id: Option<String>) -> Result<bool, String> {
    Ok(crate::llm::services::live_turns::ack_status(
        conversation_id.as_deref(),
    ))
}



#[tauri::command]
pub async fn submit_permission_decision(
    app: AppHandle,
    conversation_id: Option<String>,
    request_id: String,
    action: String,
) -> Result<bool, String> {
    let result = async {
        let parsed_action = crate::llm::utils::permissions::parse_permission_action_name(&action)
            .ok_or_else(|| format!("Unknown permission action '{}'", action))?;

        crate::llm::utils::permissions::submit_permission_decision(
            &app,
            conversation_id.as_deref(),
            &request_id,
            parsed_action,
        )
    }
    .await;
    report_backend_result(&app, "llm.client.submit_permission_decision", result, None)
}
