use tauri::AppHandle;

use crate::llm::types::{Content, ContentBlock, Message, Role};

const SESSION_RESTORE_MARKER: &str = "[Session Restore Context]";
const GLOBAL_MEMORY_MARKER: &str = "[Global Memory]";
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

fn text_from_content(content: &Content) -> String {
    match content {
        Content::Text(text) => text.trim().to_string(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn latest_user_query_text(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.role != Role::User {
            return None;
        }

        let text = text_from_content(&message.content);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn global_memory_message(app: &AppHandle, incoming: &[Message]) -> Option<Message> {
    let query = latest_user_query_text(incoming);
    let entries =
        crate::llm::services::memory_dir::relevant_global_memory(app, query.as_deref(), 8)
            .await
            .ok()?;
    if entries.is_empty() {
        return None;
    }

    let mut lines = vec![GLOBAL_MEMORY_MARKER.to_string()];

    let persistent_rules = entries
        .iter()
        .filter(|item| matches!(item.kind.as_str(), "preference" | "rule"))
        .collect::<Vec<_>>();
    let relevant_facts = entries
        .iter()
        .filter(|item| item.kind == "fact")
        .collect::<Vec<_>>();

    if !persistent_rules.is_empty() {
        lines.push("Persistent rules and preferences:".to_string());
        for (idx, item) in persistent_rules.iter().enumerate() {
            lines.push(format!("{}. [{}] {}", idx + 1, item.kind, item.content));
        }
    }

    if !relevant_facts.is_empty() {
        lines.push("Relevant facts for this request:".to_string());
        for (idx, item) in relevant_facts.iter().enumerate() {
            lines.push(format!("{}. [{}] {}", idx + 1, item.kind, item.content));
        }
    }

    Some(Message {
        role: Role::User,
        content: Content::Text(lines.join("\n")),
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

fn has_global_memory_marker(messages: &[Message]) -> bool {
    messages.iter().any(|m| match &m.content {
        Content::Text(t) => t.contains(GLOBAL_MEMORY_MARKER),
        Content::Blocks(blocks) => blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                text.contains(GLOBAL_MEMORY_MARKER)
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
// 1) 在最前面插入全局记忆 `[Global Memory]`，让模型看到跨会话偏好/规则/事实。
// 2) 当调用方显式允许 `include_session_restore` 时，才尝试插入 `[Session Restore Context]`。
//    当前正常 agent 流默认关闭它，只信任 turn snapshot，避免用摘要恢复污染模型上下文。
// 3) 可选追加环境上下文 `[AssemblerContext]`，默认关闭。
//
// 所有注入都通过 marker 做幂等检查，避免同一轮重复塞入相同类别的上下文。
pub async fn assemble_messages_for_turn(
    app: &AppHandle,
    conversation_id: Option<&str>,
    incoming: &[Message],
    options: AssembleOptions,
) -> Vec<Message> {
    // 从 query 层传入的基础上下文开始；后面只做本轮请求的临时追加/前置。
    let mut assembled = incoming.to_vec();

    // 全局记忆是跨会话的偏好/规则/事实。
    // 插到最前面，让它在本轮上下文里更像高优先级背景；如果已经存在 marker 就不重复插入。
    // 这个函数每轮都会跑，但只有当全局记忆功能有内容时才会实际插入消息；即使插入了消息，模型看到的也是带有全局记忆标记的文本，不会直接暴露底层数据结构。
    if !has_global_memory_marker(&assembled) {
        if let Some(global_msg) = global_memory_message(app, incoming).await {
            assembled.insert(0, global_msg);
        }
    }

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
    // 这个函数每轮都会跑，但只有调用方打开 include_session_restore 时才会进入此分支。
    // 当前正常 agent 流默认关闭它；保留该分支仅供显式恢复/调试入口复用。
    // 同样用 marker 防止重复注入。
    // 暂时不使用
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
    // 追加到尾部，避免盖过真实历史和记忆。
    if options.include_env_contexts {
        if let Some(msg) = env_context_message() {
            assembled.push(msg);
        }
    }
    // println!("Assembled messages for turn ({} messages):", assembled.len());
    // for (idx, msg) in assembled.iter().enumerate() {
    //     println!("  {}. [{:?}] {}", idx + 1, msg.role, text_from_content(&msg.content));
    // }
    // 返回的是本轮模型请求候选上下文；是否 compact、是否保存 snapshot 由 query 层处理。
    assembled
}
