use tauri::AppHandle;

use crate::llm::types::{Content, ContentBlock, Message, Role};

const SESSION_RESTORE_MARKER: &str = "[Session Restore Context]";
const SESSION_FILES_MARKER: &str = "[Session Files]";

#[derive(Debug, Clone, Copy)]
pub struct AssembleOptions {
    // 是否尝试插入会话恢复上下文。
    pub include_session_restore: bool,
    // 是否读取组装器自定义环境上下文（默认关闭）。
    pub include_env_contexts: bool,
}

impl Default for AssembleOptions {
    fn default() -> Self {
        Self {
            include_session_restore: true,
            include_env_contexts: false,
        }
    }
}

fn env_context_message() -> Option<Message> {
    let extra = std::env::var("NOVA_ASSEMBLER_CONTEXT").ok()?;
    let trimmed = extra.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(Message {
        role: Role::User,
        content: Content::Text(format!("[AssemblerContext] {}", trimmed)),
    })
}

// 检查消息序列里是否已经包含会话恢复标记，避免重复插入。
pub fn has_session_restore_marker(messages: &[Message]) -> bool {
    messages.iter().any(|m| match &m.content {
        Content::Text(t) => t.contains(SESSION_RESTORE_MARKER),
        Content::Blocks(blocks) => blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                text.contains(SESSION_RESTORE_MARKER)
            } else {
                false
            }
        }),
    })
}

fn has_session_files_marker(messages: &[Message]) -> bool {
    messages.iter().any(|m| match &m.content {
        Content::Text(t) => t.contains(SESSION_FILES_MARKER),
        Content::Blocks(blocks) => blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                text.contains(SESSION_FILES_MARKER)
            } else {
                false
            }
        }),
    })
}

// 组装本轮发给模型前的临时上下文；query 每轮都会调用这个函数。
//
// `incoming` 已经由 query 层决定来源：
// - 有 turn snapshot 时：snapshot + 本轮新增输入。
// - 首轮无 snapshot 时：前端当前输入。
//
// 本函数只做请求前的临时注入，不负责保存历史：
// 1) 全局记忆 frozen snapshot 现在由 system_prompt 模块注入（保持 prompt cache 稳定），
//    不再在这里每轮重检索。
// 2) 会话文件列表每轮注入（marker 防重复）。
// 3) 会话恢复上下文（include_session_restore 时）。
// 4) 可选环境上下文（include_env_contexts 时）。
pub async fn assemble_messages_for_turn(
    app: &AppHandle,
    conversation_id: Option<&str>,
    incoming: &[Message],
    options: AssembleOptions,
) -> Vec<Message> {
    let mut assembled = incoming.to_vec();

    // 会话文件列表：每轮注入，让 AI 知道有哪些文件可用，通过 Read 工具按需读取。
    // compact 后历史被压缩也能恢复文件信息。
    if !has_session_files_marker(&assembled) {
        if let Some(files_msg) =
            crate::llm::services::session_files::build_session_files_message(app, conversation_id)
                .await
        {
            assembled.push(files_msg);
        }
    }

    // 会话恢复上下文来自 compact/resume 记录。
    // 当前正常 agent 流默认关闭；保留分支仅供显式恢复/调试入口复用。
    if options.include_session_restore
        && !has_session_restore_marker(&assembled)
        && conversation_id.is_some()
    {
        if let Some(restore_msg) = crate::llm::utils::session_restore::build_resume_context_message(
            app,
            conversation_id.unwrap_or_default(),
        )
        .await
        {
            assembled.insert(0, restore_msg);
        }
    }

    // 调试/实验用的额外环境上下文，主流程默认关闭。
    if options.include_env_contexts {
        if let Some(msg) = env_context_message() {
            assembled.push(msg);
        }
    }
    assembled
}
