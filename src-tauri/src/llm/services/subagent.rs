// 子代理运行器：在独立上下文里执行只读探索任务，只把最终报告带回父对话。
//
// 设计要点：
// - 会话 ID 采用 `{parent}:sub:{uuid}` 派生格式，全链路（流事件路由 / 工具列表过滤 /
//   系统提示词分支 / 权限拦截）通过 is_subagent_conversation 识别；
// - 工具白名单（只读）：Read / Grep / Glob / GitDiff / WebSearch / WebFetch。
//   请求构建时过滤工具列表 + 执行时拦截非白名单调用，双层强制，
//   天然排除插件、MCP、写工具、Task 自身（防递归）和 ask_user_question；
// - 用量归并父会话记账（runner 在此直接以父 ID 调 log_token_usage）；
// - 并发由 Task 工具的 read_only 标记走现有批量执行器，另有信号量限制
//   同时运行的子代理数（防止一轮起飞过多烧钱）；
// - 父取消传播：每轮 provider 请求都 select 父会话的取消令牌；
// - 轮次上限 MAX_TURNS，超出时返回已积累的部分结论而非硬失败。

use std::sync::OnceLock;

use tauri::AppHandle;
use tokio::sync::Semaphore;

use crate::llm::providers::LlmClient;
use crate::llm::types::{Content, ContentBlock, Message, Role};

/// 子代理会话 ID 的分隔标记。
const SUB_MARKER: &str = ":sub:";
/// 单个子代理的最大 provider 轮数（每轮含若干工具调用）。
const MAX_TURNS: usize = 25;
/// 同时运行的子代理上限；超出者排队等待。
const MAX_CONCURRENT: usize = 3;
/// 返回给父对话的报告长度上限（字符），超出截断。
const MAX_REPORT_CHARS: usize = 16_000;

/// 子代理可用的只读工具白名单。
pub const SUBAGENT_ALLOWED_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "GitDiff",
    "WebSearch",
    "WebFetch",
];

fn concurrency_limit() -> &'static Semaphore {
    static LIMIT: OnceLock<Semaphore> = OnceLock::new();
    LIMIT.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

/// 会话 ID 是否属于子代理（格式 `{parent}:sub:{uuid}`）。
pub fn is_subagent_conversation(conversation_id: Option<&str>) -> bool {
    conversation_id
        .map(|id| id.contains(SUB_MARKER))
        .unwrap_or(false)
}

/// 从子代理会话 ID 提取父会话 ID；非子代理 ID 原样返回。
pub fn parent_conversation_id(conversation_id: &str) -> &str {
    match conversation_id.split_once(SUB_MARKER) {
        Some((parent, _)) => parent,
        None => conversation_id,
    }
}

/// 派生一个新的子代理会话 ID。
fn derive_sub_id(parent_conversation_id: &str) -> String {
    format!(
        "{}{}{}",
        parent_conversation_id,
        SUB_MARKER,
        uuid::Uuid::new_v4().simple()
    )
}

/// 子代理专属系统提示词：精简、只读导向、报告即交付物。
/// 与主提示词完全独立——不含 TodoWrite/Memory/Skills 段（对应工具不可用）。
/// 所有子代理共享同一份前缀 → Anthropic prompt cache 自动命中。
fn subagent_system_prompt() -> String {
    r#"You are a read-only research subagent. Complete the assigned task autonomously using only the available tools (Read / Grep / Glob / GitDiff / WebSearch / WebFetch).

## Rules
- Workspace root is provided in the first message. Search broadly first, then read the files that matter. Never guess content — verify with tools.
- You cannot write, edit, create files, run shell commands, or ask the user questions. Do not attempt to; if information is missing, state exactly what is missing and proceed with what you have.
- Each task is independent: do not rely on any context outside this conversation.

## Deliverable
Your final assistant message (no tool calls) IS the deliverable. The caller sees only that report.
- Lead with the answer/conclusion, then supporting evidence: file paths, line numbers, symbol names, exact code references.
- Be self-contained: include everything the caller needs to act without re-exploring.
- Be concise but complete. No filler."#
        .to_string()
}

/// 向前端推送子代理状态事件（按钮 / 侧边栏数据源）。
/// 流式增量由 stream_runner 的 subagent-event 包装转发，这里只发生命周期状态。
fn emit_status(
    app: &AppHandle,
    parent: &str,
    sub: &str,
    phase: &str,
    task: &str,
    detail: Option<String>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "subagent-event",
        serde_json::json!({
            "kind": "status",
            "parentConversationId": parent,
            "subId": sub,
            "phase": phase,
            "task": task,
            "detail": detail,
        }),
    );
}

/// 从 assistant 消息块中提取最终报告文本（text 块拼接，thinking 除外）。
fn extract_report_text(messages: &[Message]) -> String {
    let mut text = String::new();
    for message in messages {
        if message.role != Role::Assistant {
            continue;
        }
        match &message.content {
            Content::Text(t) => {
                text.push_str(t);
                text.push('\n');
            }
            Content::Blocks(blocks) => {
                for block in blocks {
                    if let ContentBlock::Text { text: t, .. } = block {
                        text.push_str(t);
                        text.push('\n');
                    }
                }
            }
        }
    }
    text.trim().to_string()
}

/// 消息里是否包含 tool_result 块（判定回合是否还要继续喂给模型）。
fn has_tool_result(messages: &[Message]) -> bool {
    messages.iter().any(|m| {
        matches!(
            &m.content,
            Content::Blocks(blocks)
                if blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        )
    })
}

/// 截断报告，保护父对话上下文。
fn truncate_report(report: String) -> String {
    if report.chars().count() <= MAX_REPORT_CHARS {
        return report;
    }
    let truncated: String = report.chars().take(MAX_REPORT_CHARS).collect();
    format!(
        "{}\n\n... [subagent report truncated, {} of {} chars shown]",
        truncated,
        MAX_REPORT_CHARS,
        report.chars().count()
    )
}

/// 运行一个子代理：独立消息列表 + 只读工具 + 有限轮数，返回最终报告。
pub async fn run(
    app: &AppHandle,
    parent_conversation_id: &str,
    task: &str,
) -> Result<String, String> {
    let _permit = concurrency_limit()
        .acquire()
        .await
        .map_err(|e| format!("subagent semaphore closed: {}", e))?;

    let sub_id = derive_sub_id(parent_conversation_id);
    emit_status(app, parent_conversation_id, &sub_id, "start", task, None);

    let workspace = crate::command::workspace::workspace_root_for_conversation(
        app,
        Some(parent_conversation_id),
    )
    .map(|root| root.display().to_string())
    .unwrap_or_default();

    // 初始消息：任务 + 工作区信息。git 摘要帮助子代理开局定位改动文件。
    let mut context_line = format!("Task:\n{}", task.trim());
    if !workspace.is_empty() {
        context_line.push_str(&format!("\n\nWorkspace root: {}", workspace));
    }
    if let Ok(root_path) = std::path::Path::new(&workspace).canonicalize() {
        if let Some(git) = crate::llm::services::git_ops::compact_git_summary(&root_path) {
            context_line.push_str(&format!("\nGit: {}", git));
        }
    }

    let mut messages = vec![Message {
        role: Role::User,
        content: Content::Text(context_line),
    }];

    let parent_cancel = crate::llm::cancellation::get_token(Some(parent_conversation_id));
    let started = std::time::Instant::now();

    let outcome = run_loop(app, &sub_id, parent_conversation_id, &mut messages, &parent_cancel).await;

    let result = match outcome {
        Ok(report) if report.is_empty() => {
            Err("subagent finished without producing a report".to_string())
        }
        Ok(report) => Ok(truncate_report(report)),
        Err(error) => Err(error),
    };

    emit_status_detail(
        app,
        parent_conversation_id,
        &sub_id,
        if result.is_ok() { "done" } else { "error" },
        task,
        result.as_ref().err().cloned(),
        result.as_ref().ok().map(|r| r.chars().take(400).collect::<String>()),
        started.elapsed(),
    );

    result
}

/// 内层循环：调用 provider（含工具执行）直到无工具调用或达到轮次上限。
async fn run_loop(
    app: &AppHandle,
    sub_id: &str,
    parent_conversation_id: &str,
    messages: &mut Vec<Message>,
    parent_cancel: &tokio_util::sync::CancellationToken,
) -> Result<String, String> {
    for turn in 0..MAX_TURNS {
        // 父会话取消：立即放弃，返回部分结果标记。
        if parent_cancel.is_cancelled() {
            let partial = extract_report_text(messages);
            return Ok(format!(
                "[cancelled by user after turn {}]\n{}",
                turn,
                if partial.is_empty() {
                    "(no partial findings)".into()
                } else {
                    partial
                }
            ));
        }

        // 每次请求前重新构建 client + 读配置：模型可能被用户切换。
        let mut provider = LlmClient::new(app)?;
        let settings = crate::command::settings::load_settings(app)?;
        let model = settings.active_provider_profile().model;

        let (provider_result, _estimate) = tokio::select! {
            result = provider.send_request(app, messages, crate::llm::types::AgentMode::Agent, Some(sub_id)) => result.map_err(|e| e.message)?,
            _ = parent_cancel.cancelled() => {
                let partial = extract_report_text(messages);
                return Ok(format!("[cancelled by user]\n{}", if partial.is_empty() { "(no partial findings)".to_string() } else { partial }));
            }
        };

        // 用量归并父会话：与主循环相同的记账字段。
        let input_tokens = provider_result
            .input_tokens
            .or(provider_result.cache_read_tokens)
            .unwrap_or(0);
        let _ = crate::llm::services::token_usage_log::log_token_usage(
            app,
            Some(parent_conversation_id),
            &model,
            None,
            input_tokens,
            provider_result.output_tokens.unwrap_or(0),
            provider_result.cache_read_tokens.unwrap_or(0),
            provider_result.cache_creation_tokens.unwrap_or(0),
            provider_result.cost.as_ref().map(|c| c.total_cost_usd.as_str()),
            Some("subagent"),
        )
        .await;

        let returned = provider_result.messages;
        if returned.is_empty() {
            return Err("subagent provider returned no messages".to_string());
        }

        let has_tools = has_tool_result(&returned);
        let report = extract_report_text(&returned);
        messages.extend(returned);

        if !has_tools {
            // 无工具调用 = 子代理交付报告，正常结束。
            return Ok(report);
        }
    }

    // 轮次耗尽：返回已积累的部分结论。
    let partial = extract_report_text(messages);
    Ok(format!(
        "[reached max turns ({}); partial findings below]\n{}",
        MAX_TURNS,
        if partial.is_empty() {
            "(no findings collected)".to_string()
        } else {
            partial
        }
    ))
}

#[allow(clippy::too_many_arguments)]
fn emit_status_detail(
    app: &AppHandle,
    parent: &str,
    sub: &str,
    phase: &str,
    task: &str,
    detail: Option<String>,
    report_preview: Option<String>,
    elapsed: std::time::Duration,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "subagent-event",
        serde_json::json!({
            "kind": "status",
            "parentConversationId": parent,
            "subId": sub,
            "phase": phase,
            "task": task,
            "detail": detail,
            "reportPreview": report_preview,
            "elapsedMs": elapsed.as_millis() as u64,
        }),
    );
}

/// 供 system_prompt 分支调用：子代理系统提示词。
pub fn system_prompt() -> String {
    subagent_system_prompt()
}
