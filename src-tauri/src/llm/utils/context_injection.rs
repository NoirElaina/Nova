//! 缓存友好的上下文注入器（对标 codex `session/world_state.rs` 的差量注入机制）。
//!
//! 核心不变量：相邻两次模型请求的消息前缀必须逐字节一致，提示词缓存才能命中。
//! 旧机制"每轮剥离注入块再重插"会让请求在上一条用户消息之后就与上轮分叉，
//! 上一轮整段助手+工具历史全部缓存失效；本模块改为：
//! 1. 注入块经事件日志（ContextMessage）持久化，写入即永久，绝不剥离；
//! 2. 每轮把当前状态渲染为块文本，与历史中最近一个同标记块的文本比对，
//!    完全一致则不注入任何内容（零成本），不同才追加新块；
//! 3. 注入块统一落在"历史末尾与新用户消息之间"，保证下一轮请求前缀
//!    恰好是上一轮末次请求的完整前缀。
//!
//! 行内排序（文件路径 / server 行）保证块文本逐字节确定，
//! 不因目录列举顺序抖动造成误判变化。

use tauri::AppHandle;

use crate::llm::types::{Content, ContentBlock, Message, Role};

/// 会话文件列表块标记。与旧机制同标记：旧块按新语义直接收编为"已注入"。
pub const SESSION_FILES_MARKER: &str = "[Session Files]";
/// MCP 服务器目录块标记。
pub const MCP_SERVER_CONTEXT_MARKER: &str = "[MCP Server Catalog]";
/// 任务阶段提示块标记。
pub const PHASE_MARKER: &str = "[Phase]";

/// 提取消息的纯文本（多 Text 块以 \n 拼接），用于标记识别与块文本比对。
fn message_text(message: &Message) -> String {
    match &message.content {
        Content::Text(text) => text.trim().to_string(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// 历史中最近一个以指定标记开头的块的文本（按编年体语义，最新状态看末尾块）。
fn last_block_text(history: &[Message], marker: &str) -> Option<String> {
    history
        .iter()
        .rev()
        .map(|message| message_text(message))
        .find(|text| text.starts_with(marker))
}

fn user_text_message(text: String) -> Message {
    Message {
        role: Role::User,
        content: Content::Text(text),
    }
}

/// 会话文件列表块：文件集合与历史末块一致时返回 None（不注入）。
async fn build_session_files_block(
    app: &AppHandle,
    conversation_id: Option<&str>,
    history: &[Message],
) -> Option<Message> {
    let Some(conv_id) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return None;
    };
    let files = match crate::llm::services::session_files::list_session_files(app, conv_id) {
        Ok(files) if !files.is_empty() => files,
        _ => return None,
    };
    let dir = match crate::llm::services::session_files::session_files_dir(app, conv_id) {
        Ok(dir) => dir,
        Err(_) => return None,
    };

    let mut lines = vec![
        SESSION_FILES_MARKER.to_string(),
        format!(
            "Uploaded files are stored at: {}",
            crate::command::workspace::display_path_string(&dir)
        ),
        "Use Read/Bash/Grep/Glob tools to access them via the absolute paths below:".to_string(),
    ];
    // 排序保证块文本逐字节确定（目录列举顺序不可靠）。
    let mut file_lines: Vec<String> = files
        .iter()
        .map(|file| {
            crate::command::workspace::display_path_string(&dir.join(&file.filename))
        })
        .collect();
    file_lines.sort();
    lines.extend(file_lines.into_iter().map(|path| format!("- {}", path)));

    let text = lines.join("\n");
    if last_block_text(history, SESSION_FILES_MARKER).as_deref() == Some(text.as_str()) {
        return None;
    }
    Some(user_text_message(text))
}

/// MCP 服务器目录块：当前集合（经智能体套件过滤）与历史末块一致时返回 None。
async fn build_mcp_catalog_block(
    app: &AppHandle,
    conversation_id: Option<&str>,
    history: &[Message],
) -> Option<Message> {
    let statuses = crate::llm::services::mcp_tools::connected_server_catalog(app).await;
    // 会话挂载的智能体套件决定可见范围：按 enabled_mcp_servers 引用清单过滤；
    // 默认 Nova（未挂载）可见全部已连接 server。
    let statuses: Vec<_> =
        match crate::llm::services::agent_bundles::active_bundle(app, conversation_id) {
            Some(bundle) => statuses
                .into_iter()
                .filter(|s| bundle.is_mcp_server_enabled(&s.name))
                .collect(),
            None => statuses,
        };
    if statuses.is_empty() {
        return None;
    }

    let mut lines = vec![
        MCP_SERVER_CONTEXT_MARKER.to_string(),
        "Connected MCP servers are available. Do not assume their internal tools up front."
            .to_string(),
        "Use `mcp_auth` with `action=\"list_tools\"` to inspect a server before calling one of its tools.".to_string(),
        "Use `mcp_auth` with `action=\"call_tool\"` to invoke a specific MCP tool after inspection.".to_string(),
    ];
    // 排序保证块文本逐字节确定。
    let mut server_lines: Vec<String> = statuses
        .iter()
        .map(|status| format!("- {} (type={}, tools={})", status.name, status.r#type, status.tool_count))
        .collect();
    server_lines.sort();
    lines.extend(server_lines);

    let text = lines.join("\n");
    if last_block_text(history, MCP_SERVER_CONTEXT_MARKER).as_deref() == Some(text.as_str()) {
        return None;
    }
    Some(user_text_message(text))
}

/// 任务阶段块：由 TodoWrite 状态推导 Explore/Execute/Verify 软编排。
/// 与历史末块一致时返回 None；分支问答与子代理会话无工具语义，永不注入。
fn build_phase_block(conversation_id: Option<&str>, history: &[Message]) -> Option<Message> {
    if crate::llm::services::branch::is_branch_conversation(conversation_id)
        || crate::llm::services::subagent::is_subagent_conversation(conversation_id)
    {
        return None;
    }

    let todos = crate::llm::tools::shared::todo_state::global_registry().list(conversation_id);
    let (phase, hint) = if todos.is_empty() {
        (
            "Explore",
            "Collect context with Read/Grep/Glob. For tasks with 3+ steps, use TodoWrite to create a task list before making changes. For trivial 1-2 step tasks, proceed directly.",
        )
    } else if todos.iter().all(|t| t.status == "completed") {
        (
            "Verify",
            "All todos completed. Run the project's test/lint/typecheck commands to verify changes. Review all uncommitted changes (e.g. `git status` and `git diff` via Bash) for completeness. Report a one-line summary of what changed and whether verification passed.",
        )
    } else {
        (
            "Execute",
            "Work through the TodoWrite list in order. Mark each item completed before starting the next. Use minimal diffs. If you discover new subtasks, update the TodoWrite list first.",
        )
    };

    let text = format!(
        "{}: {}\nPhase: {}\nCurrent todo count: {} (completed: {}, in_progress: {}, pending: {}).",
        PHASE_MARKER,
        hint,
        phase,
        todos.len(),
        todos.iter().filter(|t| t.status == "completed").count(),
        todos.iter().filter(|t| t.status == "in_progress").count(),
        todos.iter().filter(|t| t.status == "pending").count(),
    );
    if last_block_text(history, PHASE_MARKER).as_deref() == Some(text.as_str()) {
        return None;
    }
    Some(user_text_message(text))
}

/// 回合开始注入入口：依次生成会话文件 / MCP 目录 / 阶段三类块。
/// 每类都与历史中最近一个同标记块比对，未变化则不产出任何消息。
///
/// 调用方职责：把返回的消息先写 `ContextMessage` 事件、再追加到工作上下文，
/// 且必须位于本轮新用户消息之前（见 query.rs 的事件顺序约定）。
pub async fn ensure_injections(
    app: &AppHandle,
    conversation_id: Option<&str>,
    history: &[Message],
) -> Vec<Message> {
    // 顺序固定：会话文件 → MCP 目录 → 阶段，保证注入字节序跨轮稳定。
    let mut additions = Vec::new();
    if let Some(message) = build_session_files_block(app, conversation_id, history).await {
        additions.push(message);
    }
    if let Some(message) = build_mcp_catalog_block(app, conversation_id, history).await {
        additions.push(message);
    }
    if let Some(message) = build_phase_block(conversation_id, history) {
        additions.push(message);
    }
    additions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_msg(text: &str) -> Message {
        user_text_message(text.to_string())
    }

    #[test]
    fn last_block_text_picks_newest_match() {
        let history = vec![
            text_msg("[Phase] old"),
            text_msg("plain message"),
            text_msg("[Phase] new"),
        ];
        assert_eq!(
            last_block_text(&history, PHASE_MARKER).as_deref(),
            Some("[Phase] new")
        );
        assert_eq!(last_block_text(&history, SESSION_FILES_MARKER), None);
    }

    #[test]
    fn message_text_joins_blocks() {
        let message = Message {
            role: Role::User,
            content: Content::Blocks(vec![
                ContentBlock::Text {
                    text: "a".to_string(),
                },
                ContentBlock::Text {
                    text: "".to_string(),
                },
                ContentBlock::Text {
                    text: "b".to_string(),
                },
            ]),
        };
        assert_eq!(message_text(&message), "a\nb");
    }

    #[test]
    fn phase_block_skips_for_branch_and_subagent() {
        assert!(build_phase_block(Some("parent:branch:xyz"), &[]).is_none());
        assert!(build_phase_block(Some("parent:sub:xyz"), &[]).is_none());
    }

    #[test]
    fn phase_block_dedups_against_identical_history_block() {
        // 无 todo 时生成 Explore 块；用同样的文本作为历史末块时应返回 None。
        let first = build_phase_block(Some("__phase_test__"), &[]).expect("first inject");
        let again = build_phase_block(Some("__phase_test__"), &[first.clone()]);
        assert!(again.is_none());
    }
}
