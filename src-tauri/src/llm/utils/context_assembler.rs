use std::path::Path;

use tauri::AppHandle;

use crate::llm::types::{Content, ContentBlock, Message, Role};

const SESSION_RESTORE_MARKER: &str = "[Session Restore Context]";
const SESSION_FILES_MARKER: &str = "[Session Files]";
const PROJECT_CONTEXT_MARKER: &str = "[Project Context]";

// 目录列表时跳过的常见构建/依赖目录，避免注入噪音和大目录爆上下文。
const DIR_LISTING_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    ".cache",
    "coverage",
    ".idea",
    ".vscode",
    ".nova",
];

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

fn has_project_context_marker(messages: &[Message]) -> bool {
    messages.iter().any(|m| match &m.content {
        Content::Text(t) => t.contains(PROJECT_CONTEXT_MARKER),
        Content::Blocks(blocks) => blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                text.contains(PROJECT_CONTEXT_MARKER)
            } else {
                false
            }
        }),
    })
}

// 生成工作区第一层目录/文件列表，跳过构建产物和依赖目录。
// 只列一层，避免大仓库爆上下文；agent 需要更深层结构时会主动 Glob/Read。
fn build_directory_listing(root: &Path) -> String {
    let mut entries: Vec<String> = Vec::new();
    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return String::new(),
    };

    for entry in read_dir.flatten() {
        let file_name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && DIR_LISTING_SKIP_DIRS.iter().any(|skip| *skip == file_name) {
            continue;
        }
        // 标记目录便于 agent 识别结构。
        entries.push(if is_dir {
            format!("{}/", file_name)
        } else {
            file_name
        });
    }

    entries.sort();
    // 限制条数，避免超大目录爆上下文。
    let max_entries = 40usize;
    let mut out = String::new();
    for entry in entries.iter().take(max_entries) {
        out.push_str(&format!("  - {}\n", entry));
    }
    if entries.len() > max_entries {
        out.push_str(&format!("  ...and {} more\n", entries.len() - max_entries));
    }
    out
}

// 构建项目上下文消息：工作区路径 + 第一层目录结构 + git 状态摘要。
// 让 agent 每轮都能看到项目结构和当前改动，避免"盲改"。
async fn build_project_context_message(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Option<Message> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id).ok()?;
    let root_display = crate::command::workspace::display_path_string(&root);

    let mut lines = vec![
        PROJECT_CONTEXT_MARKER.to_string(),
        format!("Workspace: {}", root_display),
    ];

    let dir_listing = build_directory_listing(&root);
    if !dir_listing.is_empty() {
        lines.push("Top-level entries:".to_string());
        // 去掉末尾换行后整体 push，保持格式紧凑。
        lines.push(dir_listing.trim_end().to_string());
    } else {
        lines.push("Top-level entries: (empty or unreadable)".to_string());
    }

    if let Some(git_summary) = crate::llm::services::git_ops::workspace_git_status_summary(&root) {
        lines.push("Git status:".to_string());
        lines.push(git_summary.trim_end().to_string());
    }

    Some(Message {
        role: Role::User,
        content: Content::Text(lines.join("\n")),
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
// 2) 项目上下文（工作区结构 + git 状态）每轮注入，让 agent 看到当前改动避免盲改。
// 3) 会话文件列表每轮注入（marker 防重复）。
// 4) 会话恢复上下文（include_session_restore 时）。
// 5) 可选环境上下文（include_env_contexts 时）。
pub async fn assemble_messages_for_turn(
    app: &AppHandle,
    conversation_id: Option<&str>,
    incoming: &[Message],
    options: AssembleOptions,
) -> Vec<Message> {
    let mut assembled = incoming.to_vec();

    // 项目上下文：工作区路径 + 第一层目录 + git 状态摘要。
    // query.rs 的 strip_injected_context 每轮会剥离旧注入，这里重新填入最新状态。
    // 放在 session_files 之前，让 agent 先了解项目结构再看会话上传文件。
    if !has_project_context_marker(&assembled) {
        if let Some(project_msg) = build_project_context_message(app, conversation_id).await {
            assembled.push(project_msg);
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
