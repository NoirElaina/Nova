use tauri::AppHandle;
use crate::provider::types::{Content, Message, Role};

/// 确定性上下文装配器（Deterministic Context Assembler）
/// 严格确保 Prompt Caching 字节对齐，前缀绝对稳定，杜绝由于插入时间戳或动态标记导致缓存击穿。
pub struct ContextAssembler;

impl ContextAssembler {
    /// 组装发送给 LLM 的标准上下文
    pub async fn assemble(
        _app: &AppHandle,
        _conversation_id: Option<&str>,
        history: Vec<Message>,
        new_prompt: &str,
    ) -> Result<Vec<Message>, String> {
        let mut messages = history;
        let trimmed = new_prompt.trim();

        // 检查末尾是否已经包含了当前的用户意图（避免从 session_log 重构上下文后发生双重追加 Bug）
        let already_present = messages.last().is_some_and(|last| {
            matches!(last.role, Role::User)
                && match &last.content {
                    Content::Text(t) => t.trim() == trimmed,
                    _ => false,
                }
        });

        if !trimmed.is_empty() && !already_present {
            messages.push(Message {
                role: Role::User,
                content: Content::Text(trimmed.to_string()),
            });
        }

        Ok(messages)
    }
}
