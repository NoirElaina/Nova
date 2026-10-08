use tauri::AppHandle;
use crate::provider::types::Message;

/// 确定性上下文装配器（Deterministic Context Assembler）
pub struct ContextAssembler;

impl ContextAssembler {
    /// 组装发送给 LLM 的标准上下文
    pub async fn assemble(
        _app: &AppHandle,
        _conversation_id: Option<&str>,
        history: Vec<Message>,
    ) -> Result<Vec<Message>, String> {
        Ok(history)
    }
}
