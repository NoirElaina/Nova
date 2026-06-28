use tauri::AppHandle;

use crate::llm::tools::shared::todo_state::global_registry as todo_registry;
use crate::llm::types::{Content, ContentBlock, Message, Role};

const SESSION_RESTORE_MARKER: &str = "[Session Restore Context]";
const SESSION_FILES_MARKER: &str = "[Session Files]";
const PROJECT_CONTEXT_MARKER: &str = "[Project Context]";
const PHASE_MARKER: &str = "[Phase]";

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

fn has_phase_marker(messages: &[Message]) -> bool {
    messages.iter().any(|m| match &m.content {
        Content::Text(t) => t.contains(PHASE_MARKER),
        Content::Blocks(blocks) => blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                text.contains(PHASE_MARKER)
            } else {
                false
            }
        }),
    })
}

// 根据 TodoWrite 状态推断当前阶段，构建阶段提示消息。
// - 无 todo：Explore（让 agent 先建立清单）
// - 有 todo 但未全部完成：Execute（按清单推进）
// - 所有 todo 已完成：Verify（运行验证 + GitDiff 复查）
fn build_phase_message(conversation_id: Option<&str>) -> Option<Message> {
    let todos = todo_registry().list(conversation_id);
    let (phase, hint) = if todos.is_empty() {
        (
            "Explore",
            "Collect context with Read/Grep/Glob/GitDiff. For tasks with 3+ steps, use TodoWrite to create a task list before making changes. For trivial 1-2 step tasks, proceed directly.",
        )
    } else {
        let all_completed = todos.iter().all(|t| t.status == "completed");
        if all_completed {
            (
                "Verify",
                "All todos completed. Run the project's test/lint/typecheck commands to verify changes. Use GitDiff to review all uncommitted changes for completeness. Report a one-line summary of what changed and whether verification passed.",
            )
        } else {
            (
                "Execute",
                "Work through the TodoWrite list in order. Mark each item completed before starting the next. Use minimal diffs. If you discover new subtasks, update the TodoWrite list first.",
            )
        }
    };

    Some(Message {
        role: Role::User,
        content: Content::Text(format!(
            "{}: {}\nPhase: {}\nCurrent todo count: {} (completed: {}, in_progress: {}, pending: {}).",
            PHASE_MARKER,
            hint,
            phase,
            todos.len(),
            todos.iter().filter(|t| t.status == "completed").count(),
            todos
                .iter()
                .filter(|t| t.status == "in_progress")
                .count(),
            todos.iter().filter(|t| t.status == "pending").count(),
        )),
    })
}

// 构建项目上下文消息：仅注入工作区路径。
// 不注入目录列表和 git 状态——让 agent 自己用 Glob/Read/GitDiff 按需查看，避免杂乱信息干扰。
async fn build_project_context_message(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Option<Message> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id).ok()?;
    let root_display = crate::command::workspace::display_path_string(&root);

    Some(Message {
        role: Role::User,
        content: Content::Text(format!(
            "{}\nWorkspace: {}",
            PROJECT_CONTEXT_MARKER, root_display
        )),
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
// 3) 任务阶段（Explore/Execute/Verify）每轮按 TodoWrite 状态推断并注入。
// 4) 会话文件列表每轮注入（marker 防重复）。
// 5) 会话恢复上下文（include_session_restore 时）。
// 6) 可选环境上下文（include_env_contexts 时）。
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

    // 任务阶段提示：根据当前 TodoWrite 状态推断 Explore/Execute/Verify，
    // 注入对应阶段的行为提示词，实现 plan-execute-verify 软编排。
    if !has_phase_marker(&assembled) {
        if let Some(phase_msg) = build_phase_message(conversation_id) {
            assembled.push(phase_msg);
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
