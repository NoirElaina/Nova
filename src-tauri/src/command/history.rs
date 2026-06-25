use tauri::AppHandle;

use crate::llm::commands::{compact, memory, resume};
use crate::llm::history;
use crate::llm::utils::error_event::report_backend_result;
// 对外复用 llm/commands 公共类型。
pub use crate::llm::commands::types::{
    CompactBoundary, CompactContext, ConversationHandover, ConversationMemory, ConversationMeta,
    GlobalMemoryEntry, HistoryMessage, HistoryToolExecution, ResumeContext,
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
pub async fn append_history(
    app: AppHandle,
    conversation_id: String,
    message: HistoryMessage,
) -> Result<(), String> {
    // 向指定会话追加一条历史消息。
    report_backend_result(
        &app,
        "command.history.append_history",
        history::append_history(&app, &conversation_id, message).await,
        None,
    )
}

#[tauri::command]
pub async fn replace_history(
    app: AppHandle,
    conversation_id: String,
    messages: Vec<HistoryMessage>,
) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.history.replace_history",
        history::replace_history(&app, &conversation_id, messages).await,
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
pub async fn upsert_conversation_tool_log(
    app: AppHandle,
    conversation_id: String,
    log: HistoryToolExecution,
) -> Result<(), String> {
    report_backend_result(
        &app,
        "command.history.upsert_conversation_tool_log",
        history::upsert_conversation_tool_log(&app, &conversation_id, log).await,
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
    report_backend_result(
        &app,
        "command.history.delete_conversation",
        history::delete_conversation(&app, &conversation_id).await,
        None,
    )
}

#[tauri::command]
pub async fn get_conversation_memory(
    app: AppHandle,
    conversation_id: String,
) -> Result<Option<ConversationMemory>, String> {
    let result = async {
        // 获取带 schema 的连接池。
        let pool = history::get_pool_with_schema(&app).await?;
        // 查询会话 memory。
        memory::get_conversation_memory_by_pool(&pool, &conversation_id).await
    }
    .await;
    report_backend_result(
        &app,
        "command.history.get_conversation_memory",
        result,
        None,
    )
}

#[tauri::command]
pub async fn get_conversation_handover(
    app: AppHandle,
    conversation_id: String,
    recent_limit: Option<i64>,
) -> Result<ConversationHandover, String> {
    let result = async {
        // 获取带 schema 的连接池。
        let pool = history::get_pool_with_schema(&app).await?;
        // 计算 handover 数据。
        memory::get_conversation_handover_by_pool(&pool, &conversation_id, recent_limit).await
    }
    .await;
    report_backend_result(
        &app,
        "command.history.get_conversation_handover",
        result,
        None,
    )
}

#[tauri::command]
pub async fn get_conversation_compact_context(
    app: AppHandle,
    conversation_id: String,
    token_budget: Option<i64>,
    recent_limit: Option<i64>,
) -> Result<CompactContext, String> {
    let result = async {
        // 获取带 schema 的连接池。
        let pool = history::get_pool_with_schema(&app).await?;
        // 基于 memory 先构造 handover。
        let handover =
            memory::get_conversation_handover_by_pool(&pool, &conversation_id, recent_limit)
                .await?;
        // 在 handover 基础上构造 compact context。
        Ok(compact::build_compact_context(
            conversation_id,
            handover,
            token_budget,
            recent_limit,
        ))
    }
    .await;
    report_backend_result(
        &app,
        "command.history.get_conversation_compact_context",
        result,
        None,
    )
}

pub async fn record_compact_boundary(
    app: AppHandle,
    compact_ctx: &CompactContext,
    summary: &str,
    key_facts: &[String],
) -> Result<CompactBoundary, String> {
    // 获取带 schema 的连接池。
    let pool = history::get_pool_with_schema(&app).await?;
    // 持久化 compact 边界记录。
    compact::record_compact_boundary_by_pool(&pool, compact_ctx, summary, key_facts).await
}

#[tauri::command]
pub async fn get_latest_compact_boundary(
    app: AppHandle,
    conversation_id: String,
) -> Result<Option<CompactBoundary>, String> {
    let result = async {
        // 获取带 schema 的连接池。
        let pool = history::get_pool_with_schema(&app).await?;
        // 查询最新 compact 边界。
        compact::get_latest_compact_boundary_by_pool(&pool, &conversation_id).await
    }
    .await;
    report_backend_result(
        &app,
        "command.history.get_latest_compact_boundary",
        result,
        None,
    )
}

#[tauri::command]
pub async fn get_conversation_resume_context(
    app: AppHandle,
    conversation_id: String,
) -> Result<Option<ResumeContext>, String> {
    let result = async {
        // 获取带 schema 的连接池。
        let pool = history::get_pool_with_schema(&app).await?;
        // 先找最新 compact 边界。
        let boundary =
            match compact::get_latest_compact_boundary_by_pool(&pool, &conversation_id).await? {
                Some(v) => v,
                // 无边界则无 resume 上下文。
                None => return Ok(None),
            };
        // 计算边界之后的 resume 上下文。
        let ctx =
            resume::get_conversation_resume_context_by_pool(&pool, &conversation_id, boundary)
                .await?;
        // 包装为 Some 返回。
        Ok(Some(ctx))
    }
    .await;
    report_backend_result(
        &app,
        "command.history.get_conversation_resume_context",
        result,
        None,
    )
}

#[tauri::command]
pub async fn upsert_conversation_memory(
    app: AppHandle,
    conversation_id: String,
    summary: String,
    key_facts: Vec<String>,
) -> Result<(), String> {
    let result = async {
        // 获取带 schema 的连接池。
        let pool = history::get_pool_with_schema(&app).await?;
        // upsert 会话 memory。
        memory::upsert_conversation_memory_by_pool(&pool, &conversation_id, &summary, &key_facts)
            .await
    }
    .await;
    report_backend_result(
        &app,
        "command.history.upsert_conversation_memory",
        result,
        None,
    )
}

#[tauri::command]
pub async fn list_global_memory(
    app: AppHandle,
    limit: Option<i64>,
) -> Result<Vec<GlobalMemoryEntry>, String> {
    report_backend_result(
        &app,
        "command.history.list_global_memory",
        history::list_global_memory(&app, limit).await,
        None,
    )
}

#[tauri::command]
pub async fn upsert_global_memory(
    app: AppHandle,
    content: String,
    kind: Option<String>,
    source: Option<String>,
) -> Result<GlobalMemoryEntry, String> {
    report_backend_result(
        &app,
        "command.history.upsert_global_memory",
        history::upsert_global_memory(&app, &content, kind.as_deref(), source.as_deref()).await,
        None,
    )
}

#[tauri::command]
pub async fn delete_global_memory(app: AppHandle, id: String) -> Result<bool, String> {
    let result = async {
        let parsed_id = id
            .trim()
            .parse::<i64>()
            .map_err(|e| format!("invalid global memory id '{}': {}", id, e))?;
        history::delete_global_memory(&app, parsed_id).await
    }
    .await;
    report_backend_result(&app, "command.history.delete_global_memory", result, None)
}

#[tauri::command]
pub async fn clear_global_memory(app: AppHandle) -> Result<i64, String> {
    report_backend_result(
        &app,
        "command.history.clear_global_memory",
        history::clear_global_memory(&app).await,
        None,
    )
}
