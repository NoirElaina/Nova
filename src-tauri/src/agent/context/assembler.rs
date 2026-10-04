use tauri::AppHandle;
use crate::llm::types::{Content, Message, Role};

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

        // 最新用户意图严格以 User Message 追加在尾部，不污染历史前缀
        if !new_prompt.trim().is_empty() {
            messages.push(Message {
                role: Role::User,
                content: Content::Text(new_prompt.trim().to_string()),
            });
        }

        Ok(messages)
    }
}
