
use crate::agent::session::history;
use crate::agent::utils::error_event::report_backend_result;
// 对外复用 llm/commands 公共类型。
pub use crate::agent::session::types::{
    ConversationMeta, HistoryMessage, HistoryToolExecution,
};

pub async fn create_conversation(
    core: &crate::core::Core,
    title: Option<String>,
    workspace_path: Option<String>,
    idempotency_key: Option<String>,
) -> Result<ConversationMeta, crate::NovaCoreError> {
    let outcome = core
        .run_idempotent("create_conversation", idempotency_key.as_deref(), || async {
            history::create_conversation(core, title, workspace_path)
                .await
                .map_err(crate::NovaCoreError::Session)
        })
        .await;
    report_backend_result(
        core,
        "command.history.create_conversation",
        outcome,
        None,
    )
}

pub async fn list_conversations(core: &crate::core::Core) -> Result<Vec<ConversationMeta>, String> {
    // 拉取会话列表。
    report_backend_result(
        core,
        "command.history.list_conversations",
        history::list_conversations(core).await,
        None,
    )
}

pub async fn set_conversation_pinned(
    core: &crate::core::Core,
    conversation_id: String,
    pinned: bool,
) -> Result<(), String> {
    report_backend_result(
        core,
        "command.history.set_conversation_pinned",
        history::set_conversation_pinned(core, &conversation_id, pinned).await,
        None,
    )
}

pub async fn export_conversation(
    core: &crate::core::Core,
    conversation_id: String,
    format: String,
) -> Result<String, String> {
    report_backend_result(
        core,
        "command.history.export_conversation",
        history::export_conversation(core, &conversation_id, &format).await,
        None,
    )
}

pub async fn export_rendered_conversation_pdf(
    core: &crate::core::Core,
    conversation_id: String,
    title: String,
    html: String,
) -> Result<String, String> {
    report_backend_result(
        core,
        "command.history.export_rendered_conversation_pdf",
        history::export_rendered_conversation_pdf(core, &conversation_id, &title, &html).await,
        None,
    )
}

pub async fn load_history(
    core: &crate::core::Core,
    conversation_id: String,
) -> Result<Vec<HistoryMessage>, String> {
    // 加载指定会话历史。
    report_backend_result(
        core,
        "command.history.load_history",
        history::load_history(core, &conversation_id).await,
        None,
    )
}

pub async fn replace_history(
    core: &crate::core::Core,
    conversation_id: String,
    messages: Vec<HistoryMessage>,
) -> Result<(), String> {
    // 事件日志时代：重置该会话事件流为传入的消息序列（编辑消息重发用）。
    report_backend_result(
        core,
        "command.history.replace_history",
        history::reset_conversation_messages(core, &conversation_id, &messages).await,
        None,
    )
}

pub async fn append_plain_chat_message(
    core: &crate::core::Core,
    conversation_id: String,
    role: String,
    content: String,
) -> Result<(), String> {
    // 事件日志入口：追加一条纯文本消息（分支转存等场景）。
    report_backend_result(
        core,
        "command.history.append_plain_chat_message",
        history::append_plain_chat_message(core, &conversation_id, &role, &content).await,
        None,
    )
}


pub async fn load_conversation_tool_logs(
    core: &crate::core::Core,
    conversation_id: String,
) -> Result<Vec<HistoryToolExecution>, String> {
    report_backend_result(
        core,
        "command.history.load_conversation_tool_logs",
        history::load_conversation_tool_logs(core, &conversation_id).await,
        None,
    )
}

pub async fn get_conversation_turn_traces(
    core: &crate::core::Core,
    conversation_id: String,
) -> Result<Vec<crate::agent::session::projection::TurnTrace>, String> {
    // 调试面板：从事件流投影出按回合分组的完整交互轨迹。
    let result = async {
        let events = crate::agent::session::load_events(core, &conversation_id).await?;
        Ok(crate::agent::session::projection::render_turn_traces(&events))
    }
    .await;
    report_backend_result(
        core,
        "command.history.get_conversation_turn_traces",
        result,
        None,
    )
}

pub async fn clear_history(core: &crate::core::Core, conversation_id: Option<String>) -> Result<(), String> {
    // 清理指定会话或全部会话历史。
    report_backend_result(
        core,
        "command.history.clear_history",
        history::clear_history(core, conversation_id).await,
        None,
    )
}

pub async fn delete_conversation(core: &crate::core::Core, conversation_id: String) -> Result<(), String> {
    // 删除指定会话及其附属数据。
    let result = report_backend_result(
        core,
        "command.history.delete_conversation",
        history::delete_conversation(core, &conversation_id).await,
        None,
    );
    if result.is_ok() {
        // 联动清理该会话的渐进式工具披露状态（内存缓存 + 磁盘记录）。
        crate::agent::capabilities::tool_disclosure::forget_conversation(core, Some(&conversation_id));
    }
    result
}

pub async fn list_memory_entries(core: &crate::core::Core) -> Result<Vec<String>, String> {
    report_backend_result(
        core,
        "command.history.list_memory_entries",
        crate::agent::capabilities::memory_dir::memory_list(core).await,
        None,
    )
}

pub async fn add_memory_entry(core: &crate::core::Core, content: String) -> Result<(), String> {
    report_backend_result(
        core,
        "command.history.add_memory_entry",
        crate::agent::capabilities::memory_dir::memory_add(core, &content).await,
        None,
    )
}

pub async fn remove_memory_entry(core: &crate::core::Core, old_text: String) -> Result<(), String> {
    report_backend_result(
        core,
        "command.history.remove_memory_entry",
        crate::agent::capabilities::memory_dir::memory_remove(core, &old_text).await,
        None,
    )
}

pub async fn clear_memory_entries(core: &crate::core::Core) -> Result<(), String> {
    report_backend_result(
        core,
        "command.history.clear_memory_entries",
        crate::agent::capabilities::memory_dir::memory_clear(core).await,
        None,
    )
}


pub async fn manual_compact_conversation(
    core: &crate::core::Core,
    conversation_id: String,
) -> Result<crate::agent::capabilities::compact::ManualCompactOutcome, String> {
    report_backend_result(
        core,
        "command.history.manual_compact_conversation",
        crate::agent::capabilities::compact::manual_compact(core, &conversation_id).await,
        None,
    )
}
