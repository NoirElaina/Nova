// 子代理运行器：通过 Agent 认知调度内核统一执行只读探索任务，将最终报告带回父对话。
//
// 设计要点：
// - 会话 ID 采用 `{parent}:sub:{uuid}` 派生格式，全链路（流事件路由 / 工具列表过滤 /
//   系统提示词分支 / 权限拦截）通过 is_subagent_conversation 识别；
// - 工具白名单（只读）：Read / Grep / Glob / WebSearch / WebFetch。
//   系统级白名单强制过滤，天然排除 MCP、写工具、Task 自身（防递归）和 ask_user_question；
// - 统一调度架构：直接复用 AgentEngine 完整的 ReAct 认知闭环、自愈机制与死循环熔断器；
// - 用量归并父会话记账；
// - 并发由信号量限制同时运行的子代理数（MAX_CONCURRENT）；
// - 取消令牌双向级联穿透。

use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::sync::Semaphore;

/// 子代理会话 ID 的分隔标记。
const SUB_MARKER: &str = ":sub:";
/// 同时运行的子代理上限；超出者排队等待。
const MAX_CONCURRENT: usize = 3;
/// 返回给父对话的报告长度上限（字符），超出截断。
const MAX_REPORT_CHARS: usize = 16_000;

/// 子代理可用的只读工具白名单。
pub const SUBAGENT_ALLOWED_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
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
    r#"You are a read-only research subagent. Complete the assigned task autonomously using only the available tools (Read / Grep / Glob / WebSearch / WebFetch).

## Rules
- Workspace root is provided in the first message. Search broadly first, then read the files that matter. Never guess content — verify with tools.
- Read can only read individual files — to see what a directory contains, use Glob patterns (e.g. `**/*.rs`, `src/*`), never Read on a directory path.
- You cannot write, edit, create files, run shell commands, or ask the user questions. Do not attempt to; if information is missing, state practical findings and proceed with what you have.
- Each task is independent: do not rely on any context outside this conversation.

## Deliverable
Your final assistant message (no tool calls) IS the deliverable. The caller sees only that report.
- Lead with the answer/conclusion, then supporting evidence: file paths, line numbers, symbol names, exact code references.
- Be self-contained: include everything the caller needs to act without re-exploring.
- Be concise but complete. No filler."#
        .to_string()
}

/// 向前端推送子代理状态事件（生命周期数据源）。
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

/// 运行一个子代理：借助 Agent 核心引擎执行完整 ReAct 闭环，返回最终交付报告。
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

    let parent_cancel = crate::agent::cancellation::get_token(Some(parent_conversation_id));
    let started = std::time::Instant::now();

    // 级联父会话取消令牌
    let sub_cancel = tokio_util::sync::CancellationToken::new();
    let sub_cancel_clone = sub_cancel.clone();
    let parent_cancel_clone = parent_cancel.clone();
    let join_handle = tokio::spawn(async move {
        tokio::select! {
            _ = parent_cancel_clone.cancelled() => {
                sub_cancel_clone.cancel();
            }
            _ = sub_cancel_clone.cancelled() => {}
        }
    });

    // 直接复用 AgentEngine 认知引擎执行 ReAct 闭环
    let engine = crate::agent::engine::AgentEngine::new(app.clone());
    let outcome = engine.execute_turn(&sub_id, task, None, sub_cancel.clone()).await;
    sub_cancel.cancel();
    let _ = join_handle.await;

    // 清理子会话存储的临时事件，保持持久化存储轻量纯净
    let _ = crate::agent::session::delete_events(app, &sub_id).await;

    let result = match outcome {
        Ok(turn_outcome) => {
            let report = turn_outcome.assistant_text.trim();
            if report.is_empty() {
                if turn_outcome.stop_reason == "cancelled" {
                    Ok("[subagent cancelled by user]".to_string())
                } else {
                    Err("subagent finished without producing a report".to_string())
                }
            } else {
                Ok(truncate_report(report.to_string()))
            }
        }
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
