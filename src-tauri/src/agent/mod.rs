pub mod context;
pub mod engine;
pub mod events;
pub mod state;
pub mod tools;

use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

/// 现代化 Agent 外部统一调用入口
#[tauri::command]
pub async fn send_modern_agent_turn(
    app: AppHandle,
    conversation_id: String,
    prompt: String,
) -> Result<(), String> {
    let engine = engine::AgentEngine::new(app);
    let cancel_token = CancellationToken::new();
    engine.execute_turn(&conversation_id, &prompt, cancel_token).await
}
