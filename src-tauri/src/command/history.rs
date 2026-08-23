use tauri::AppHandle;

use crate::llm::history;
use crate::llm::utils::error_event::report_backend_result;
// 对外复用 llm/commands 公共类型。
pub use crate::llm::commands::types::{
    ConversationMeta, HistoryMessage, HistoryToolExecution,
};

#[tauri::command]
pub async fn create_conversation(
    app: AppHandle,
    title: Option<String>,
    workspace_path: Option<String>,
) -> Result<ConversationMeta, String> {
    // 直接转发到 history 服务创建会话；workspace_path 为空时由 history 用内置默认工作区。
    report_backend_result(
        &app,
        "command.history.create_conversation",
        history::create_conversation(&app, title, workspace_path).await,
        None,
    )
}

#[tauri::command]
pub async fn list_conversations(app: AppHandle) -> Result<Vec<ConversationMeta>, String> {
    // 拉取会话列表。
    report_backend_result(
        &app,
        "command.history.list_conversations",
        history::list_conversations(&app).await,
        None,
    )
}

#[tauri::command]
pub async fn set_conversation_pinned(
    app: AppHandle,
    conversation_id: String,
    pinned: bool,
) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.history.set_conversation_pinned",
        history::set_conversation_pinned(&app, &conversation_id, pinned).await,
        None,
    )
}

#[tauri::command]
pub async fn export_conversation(
    app: AppHandle,
    conversation_id: String,
    format: String,
) -> Result<String, String> {
    report_backend_result(
        &app,
        "command.history.export_conversation",
        history::export_conversation(&app, &conversation_id, &format).await,
        None,
    )
}

#[tauri::command]
pub async fn export_rendered_conversation_pdf(
    app: AppHandle,
    conversation_id: String,
    title: String,
    html: String,
) -> Result<String, String> {
    report_backend_result(
        &app,
        "command.history.export_rendered_conversation_pdf",
        history::export_rendered_conversation_pdf(&app, &conversation_id, &title, &html).await,
        None,
    )
}

#[tauri::command]
pub async fn load_history(
    app: AppHandle,
    conversation_id: String,
) -> Result<Vec<HistoryMessage>, String> {
    // 加载指定会话历史。
    report_backend_result(
        &app,
        "command.history.load_history",
        history::load_history(&app, &conversation_id).await,
        None,
    )
}

#[tauri::command]
pub async fn replace_history(
    app: AppHandle,
    conversation_id: String,
    messages: Vec<HistoryMessage>,
) -> Result<(), String> {
    // 事件日志时代：重置该会话事件流为传入的消息序列（编辑消息重发用）。
    report_backend_result(
        &app,
        "command.history.replace_history",
        history::reset_conversation_messages(&app, &conversation_id, &messages).await,
        None,
    )
}

#[tauri::command]
pub async fn append_plain_chat_message(
    app: AppHandle,
    conversation_id: String,
    role: String,
    content: String,
) -> Result<(), String> {
    // 事件日志入口：追加一条纯文本消息（分支转存等场景）。
    report_backend_result(
        &app,
        "command.history.append_plain_chat_message",
        history::append_plain_chat_message(&app, &conversation_id, &role, &content).await,
        None,
    )
}

#[tauri::command]
pub async fn update_assistant_message_meta(
    app: AppHandle,
    conversation_id: String,
    cost: serde_json::Value,
) -> Result<(), String> {
    // 前端回写助手消息的展示元数据（transcript/压缩记录/耗时等），
    // 富化事件日志里最后一条 assistant_message 的 cost 字段。
    report_backend_result(
        &app,
        "command.history.update_assistant_message_meta",
        crate::llm::session_log::update_last_assistant_message_cost(
            &app,
            &conversation_id,
            &cost,
        )
        .await,
        None,
    )
}

#[tauri::command]
pub async fn load_conversation_tool_logs(
    app: AppHandle,
    conversation_id: String,
) -> Result<Vec<HistoryToolExecution>, String> {
    report_backend_result(
        &app,
        "command.history.load_conversation_tool_logs",
        history::load_conversation_tool_logs(&app, &conversation_id).await,
        None,
    )
}

#[tauri::command]
pub async fn clear_history(app: AppHandle, conversation_id: Option<String>) -> Result<(), String> {
    // 清理指定会话或全部会话历史。
    report_backend_result(
        &app,
        "command.history.clear_history",
        history::clear_history(&app, conversation_id).await,
        None,
    )
}

#[tauri::command]
pub async fn delete_conversation(app: AppHandle, conversation_id: String) -> Result<(), String> {
    // 删除指定会话及其附属数据。
    let result = report_backend_result(
        &app,
        "command.history.delete_conversation",
        history::delete_conversation(&app, &conversation_id).await,
        None,
    );
    if result.is_ok() {
        // 联动清理该会话的渐进式工具披露状态（内存缓存 + 磁盘记录）。
        crate::llm::services::tool_disclosure::forget_conversation(&app, Some(&conversation_id));
    }
    result
}

#[tauri::command]
pub async fn list_memory_entries(app: AppHandle) -> Result<Vec<String>, String> {
    report_backend_result(
        &app,
        "command.history.list_memory_entries",
        history::list_memory_entries(&app).await,
        None,
    )
}

#[tauri::command]
pub async fn add_memory_entry(app: AppHandle, content: String) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.history.add_memory_entry",
        history::add_memory_entry(&app, &content).await,
        None,
    )
}

#[tauri::command]
pub async fn remove_memory_entry(app: AppHandle, old_text: String) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.history.remove_memory_entry",
        history::remove_memory_entry(&app, &old_text).await,
        None,
    )
}

#[tauri::command]
pub async fn clear_memory_entries(app: AppHandle) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.history.clear_memory_entries",
        history::clear_memory_entries(&app).await,
        None,
    )
}

#[tauri::command]
pub async fn manual_compact_conversation(
    app: AppHandle,
    conversation_id: String,
) -> Result<crate::llm::services::compact::ManualCompactOutcome, String> {
    report_backend_result(
        &app,
        "command.history.manual_compact_conversation",
        crate::llm::services::compact::manual_compact(&app, &conversation_id).await,
        None,
    )
}
