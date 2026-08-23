//! 会话级缓存统一清理注册表。
//!
//! 所有 per-conversation 的进程内缓存（文件读取状态 / TodoWrite 清单 /
//! 工作区路径 / 智能体挂载 / 缓存击穿检测基线）在会话删除时必须经此单一入口
//! 统一回收，杜绝"只增不减"的内存残留。
//!
//! 对齐现状说明：`services/tool_disclosure` 自带会话删除联动清理（语义自治），
//! 不在此重复挂接。

/// 清理指定会话的全部进程内缓存；`None` 表示"清除全部会话"路径，全量清空。
pub fn clear_conversation_caches(conversation_id: Option<&str>) {
    crate::llm::tools::shared::read_state::clear_conversation(conversation_id);
    crate::llm::tools::shared::todo_state::global_registry().clear_session(conversation_id);
    crate::command::workspace::evict_conversation_workspace(conversation_id);
    crate::llm::services::agent_bundles::evict_conversation_agent_cache(conversation_id);
    match conversation_id {
        Some(id) => crate::llm::services::prompt_cache_break::forget_conversation(id),
        None => crate::llm::services::prompt_cache_break::clear_all(),
    }
}
