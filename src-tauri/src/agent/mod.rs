pub mod context;
pub mod engine;
pub mod events;
pub mod state;

use tauri::AppHandle;

/// 现代化 Agent 外部统一调用入口
#[tauri::command]
pub async fn send_modern_agent_turn(
    app: AppHandle,
    conversation_id: String,
    prompt: String,
) -> Result<(), String> {
    if !crate::llm::services::live_turns::begin_turn(Some(&conversation_id)) {
        return Err("该会话已有正在进行的回复，请等待其完成或先停止。".to_string());
    }
    crate::llm::cancellation::begin_turn(Some(&conversation_id));
    let cancel_token = crate::llm::cancellation::get_token(Some(&conversation_id));
    let engine = engine::AgentEngine::new(app);
    let result = engine.execute_turn(&conversation_id, &prompt, cancel_token).await;
    crate::llm::cancellation::finish_turn(Some(&conversation_id));
    crate::llm::services::live_turns::mark_terminal(
        Some(&conversation_id),
        if result.is_ok() { "completed" } else { "error" },
    );
    result
}

/// 现代化 Agent 外部取消统一调用入口
#[tauri::command]
pub async fn cancel_modern_agent_turn(conversation_id: Option<String>) -> Result<bool, String> {
    Ok(crate::llm::cancellation::request_cancel(conversation_id.as_deref()))
}
