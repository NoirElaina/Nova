use tauri::AppHandle;
use tracing::{info, warn};

use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::error_event::report_backend_result;

// 对外复用 query_engine 的事件类型定义。
pub use crate::llm::query_engine::ChatMessageEvent;

#[tauri::command]
pub async fn send_chat_message(
    app: AppHandle,
    conversation_id: Option<String>,
    messages: Vec<Message>,
    plan_mode: Option<bool>,
    agent_mode: Option<AgentMode>,
) -> Result<(), String> {
    // 克隆会话 ID，便于请求前后使用同一作用域 key。
    let conversation_scope = conversation_id.clone();
    // 标记本轮开始，初始化取消标志位。
    crate::llm::cancellation::begin_turn(conversation_scope.as_deref());

    // 兼容旧参数：未显式提供 agent_mode 时退化到 plan_mode 开关语义。
    let resolved_mode = agent_mode.unwrap_or_else(|| {
        if plan_mode.unwrap_or(false) {
            AgentMode::Plan
        } else {
            AgentMode::Agent
        }
    });

    info!(
        conversation_id = %conversation_scope.as_deref().unwrap_or("__default__"),
        agent_mode = ?resolved_mode,
        message_count = messages.len(),
        "chat turn started"
    );

    // 通过兼容层入口发送请求。
    let result =
        crate::llm::query_engine::send_chat_message(app, conversation_id, messages, resolved_mode)
            .await;

    // 无论请求成功失败都结束本轮，清理取消状态。
    crate::llm::cancellation::finish_turn(conversation_scope.as_deref());
    match &result {
        Ok(()) => info!(
            conversation_id = %conversation_scope.as_deref().unwrap_or("__default__"),
            "chat turn finished"
        ),
        Err(error) => warn!(
            conversation_id = %conversation_scope.as_deref().unwrap_or("__default__"),
            error = %error,
            "chat turn failed"
        ),
    }
    // 返回下游执行结果。
    result
}

#[tauri::command]
pub async fn cancel_chat_message(conversation_id: Option<String>) -> Result<bool, String> {
    // 提交取消请求并返回是否成功命中运行中的会话。
    let hit = crate::llm::cancellation::request_cancel(conversation_id.as_deref());
    if hit {
        info!(
            conversation_id = %conversation_id.as_deref().unwrap_or("__default__"),
            "chat turn cancel requested"
        );
    } else {
        warn!(
            conversation_id = %conversation_id.as_deref().unwrap_or("__default__"),
            "chat turn cancel missed active scope"
        );
    }
    Ok(hit)
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
            conversation_id.as_deref(),
            &request_id,
            parsed_action,
        )
    }
    .await;
    report_backend_result(&app, "llm.client.submit_permission_decision", result, None)
}
