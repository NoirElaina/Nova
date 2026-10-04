pub mod context;
pub mod engine;
pub mod events;
pub mod state;
pub mod tools;

use tauri::AppHandle;

/// 现代化 Agent 外部统一调用入口
#[tauri::command]
pub async fn send_modern_agent_turn(
    app: AppHandle,
    conversation_id: String,
    prompt: String,
) -> Result<(), String> {
    crate::llm::cancellation::begin_turn(Some(&conversation_id));
    let cancel_token = crate::llm::cancellation::get_token(Some(&conversation_id));
    let engine = engine::AgentEngine::new(app);
    let result = engine.execute_turn(&conversation_id, &prompt, cancel_token).await;
    crate::llm::cancellation::finish_turn(Some(&conversation_id));
    result
}
