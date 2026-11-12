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
use crate::core::Core;
use tokio::sync::Semaphore;

/// 子代理会话 ID 的分隔标记。
const SUB_MARKER: &str = ":sub:";
/// 同时运行的子代理上限；超出者排队等待。
const MAX_CONCURRENT: usize = 3;
/// 单个父会话内最多同时存在的活跃（PendingInit / Running）子智能体数。
pub const MAX_SUBAGENTS_PER_CONVERSATION: usize = 8;
/// 返回给父对话的报告长度上限（字符），超出截断。
const MAX_REPORT_CHARS: usize = 16_000;

/// 子代理可用的只读工具白名单。
pub const SUBAGENT_ALLOWED_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "WebSearch",
    "WebFetch",
    "board_post",
    "board_read",
    "board_list_channels",
    "send_message",
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
    r#"You are a read-only research subagent. Complete the assigned task autonomously using only the available tools:
- Exploration tools: Read, Grep, Glob, WebSearch, WebFetch
- Collaboration tools: board_post, board_read, board_list_channels

## Rules
- Workspace root is provided in the first message. Search broadly first, then read the files that matter. Never guess content — verify with tools.
- Read can only read individual files — to see what a directory contains, use Glob patterns (e.g. `**/*.rs`, `src/*`), never Read on a directory path.
- You cannot write, edit, create files, run shell commands, or ask the user questions. Do not attempt to; if information is missing, state practical findings and proceed with what you have.
- Each task is independent: do not rely on any context outside this conversation.

## Efficiency & Pacing
- Be targeted: explore top-down (Glob -> check entry points like `mod.rs` or `lib.rs` -> Grep key symbols -> Read 3-8 key definitions).
- Do NOT get trapped in an endless loop attempting to read every single file in large directories. Synthesize the core structure and patterns within 10-15 tool calls max.
- When you have gathered sufficient evidence to answer the core goal, conclude and write your final report immediately.

## Collaboration Board
- You have access to the agent collaboration message board (`board_post`, `board_read`, `board_list_channels`).
- Channels: `#general`, `#research`, `#evidence`.
- When you discover key architectural patterns, critical evidence, or milestone findings (or when instructed by the task to post to a channel such as `#research`), proactively call `board_post` to share your findings to that channel so other agents and the user see them in real-time on the collaboration board.

## Deliverable
Your final assistant message (no tool calls) IS the deliverable. The caller sees only that report.
- Lead with the answer/conclusion, then supporting evidence: file paths, line numbers, symbol names, exact code references.
- Be self-contained: include everything the caller needs to act without re-exploring.
- Be concise but complete. No filler."#
        .to_string()
}

/// 向前端推送子代理状态事件（生命周期数据源）。
fn emit_status(
    core: &Core,
    parent: &str,
    sub: &str,
    phase: &str,
    task: &str,
    detail: Option<String>,
) {
    core.emit(crate::events::NovaEvent::SubagentStatus {
        parent_conversation_id: parent.to_string(),
        sub_id: sub.to_string(),
        phase: phase.to_string(),
        task: task.to_string(),
        detail,
        report_preview: None,
        elapsed_ms: None,
    });
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
///
/// 排队期间收到中断/取消时收尾图谱与唤醒等待者，绝不再启动 `AgentEngine` 或覆盖 `Interrupted` 状态。
fn finalize_cancelled_queued(
    core: &Core,
    parent_id: &str,
    sub_id: &str,
    task: &str,
    status_lock: &std::sync::RwLock<super::registry::AgentLifecycleStatus>,
    notify_tx: &tokio::sync::watch::Sender<u64>,
) {
    if let Ok(mut st) = status_lock.write() {
        if !st.is_final() {
            *st = super::registry::AgentLifecycleStatus::Interrupted;
        }
    }
    if let Some(updated_node) = super::graph::global_graph_store().set_node_status(
        parent_id,
        sub_id,
        super::graph::ThreadSpawnEdgeStatus::Closed,
        Some("[subagent interrupted while queued]".to_string()),
    ) {
        core.emit(crate::events::NovaEvent::AgentGraphUpdated {
            conversation_id: parent_id.to_string(),
            node: serde_json::to_value(&updated_node).unwrap_or_default(),
        });
    }
    emit_status_detail(
        core,
        parent_id,
        sub_id,
        "done",
        task,
        Some("interrupted while queued".to_string()),
        None,
        std::time::Duration::ZERO,
    );
    notify_tx.send_modify(|val| *val = val.saturating_add(1));
}

/// 在已拿到执行名额后，检查是否已被取消/中断；若未取消则原子置为 `Running` 并返回 `true`。
fn transition_pending_to_running(
    cancel: &tokio_util::sync::CancellationToken,
    status_lock: &std::sync::RwLock<super::registry::AgentLifecycleStatus>,
) -> bool {
    let mut st = status_lock.write().unwrap_or_else(|e| e.into_inner());
    if cancel.is_cancelled() || st.is_final() {
        if !st.is_final() {
            *st = super::registry::AgentLifecycleStatus::Interrupted;
        }
        return false;
    }
    *st = super::registry::AgentLifecycleStatus::Running;
    true
}

/// 非阻塞异步派发子智能体：在后台 Tokio 任务中执行完整 ReAct 闭环，
/// 立即向主代理返回子节点句柄与状态，支持 Mailbox 与集群治理。
pub async fn spawn_async(
    core: &Core,
    parent_conversation_id: &str,
    task: &str,
    role: Option<&str>,
    nickname: Option<&str>,
) -> Result<serde_json::Value, String> {
    let sub_id = derive_sub_id(parent_conversation_id);
    let now = chrono::Utc::now().timestamp_millis();
    let effective_role = role.unwrap_or("researcher");
    let effective_nickname = nickname.unwrap_or("ResearchSubagent");

    let parent_cancel = crate::agent::cancellation::get_token(Some(parent_conversation_id));
    let sub_cancel = tokio_util::sync::CancellationToken::new();

    let (notify_tx, notify_rx) = tokio::sync::watch::channel(0u64);
    // 并发配额只有 3 个；超出者在 acquire 前处于 PendingInit，
    // 避免"排队中"的子智能体在 list_agents / 看板上被误报成 running。
    let status_ref = std::sync::Arc::new(std::sync::RwLock::new(
        super::registry::AgentLifecycleStatus::PendingInit,
    ));

    let handle = std::sync::Arc::new(super::registry::AgentHandle {
        id: sub_id.clone(),
        parent_id: parent_conversation_id.to_string(),
        role: effective_role.to_string(),
        nickname: effective_nickname.to_string(),
        task: task.to_string(),
        status: status_ref.clone(),
        cancel_token: sub_cancel.clone(),
        mailbox: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::VecDeque::new())),
        mailbox_notify: notify_tx.clone(),
        mailbox_rx: notify_rx,
        created_at: now,
    });
    if let Err(active) = super::registry::global_agent_registry()
        .try_register(handle, MAX_SUBAGENTS_PER_CONVERSATION)
    {
        return Err(format!(
            "Maximum active subagents reached for this conversation ({}/{}). Wait for or interrupt active subagents before spawning more.",
            active, MAX_SUBAGENTS_PER_CONVERSATION
        ));
    }

    let initial_node = super::graph::AgentNode {
        id: sub_id.clone(),
        parent_id: Some(parent_conversation_id.to_string()),
        role: effective_role.to_string(),
        nickname: effective_nickname.to_string(),
        task: task.to_string(),
        status: super::graph::ThreadSpawnEdgeStatus::Open,
        created_at: now,
        updated_at: now,
        result_preview: None,
    };
    super::graph::global_graph_store().upsert_node(parent_conversation_id, initial_node.clone());
    core.emit(crate::events::NovaEvent::AgentGraphUpdated {
        conversation_id: parent_conversation_id.to_string(),
        node: serde_json::to_value(&initial_node).unwrap_or_default(),
    });

    emit_status(core, parent_conversation_id, &sub_id, "start", task, None);

    // 级联父会话取消令牌
    let sub_cancel_clone = sub_cancel.clone();
    let parent_cancel_clone = parent_cancel.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = parent_cancel_clone.cancelled() => {
                sub_cancel_clone.cancel();
            }
            _ = sub_cancel_clone.cancelled() => {}
        }
    });

    // 启动后台自主协程运行该子代理
    let bg_core = core.clone();
    let bg_parent_id = parent_conversation_id.to_string();
    let bg_sub_id = sub_id.clone();
    let bg_task = task.to_string();
    let bg_role = effective_role.to_string();
    let bg_nickname = effective_nickname.to_string();
    let bg_sub_cancel = sub_cancel.clone();
    let bg_status_ref = status_ref.clone();
    let bg_notify_tx = notify_tx;

    tokio::spawn(async move {
        let _permit = tokio::select! {
            biased;
            _ = bg_sub_cancel.cancelled() => {
                finalize_cancelled_queued(
                    &bg_core,
                    &bg_parent_id,
                    &bg_sub_id,
                    &bg_task,
                    &bg_status_ref,
                    &bg_notify_tx,
                );
                return;
            }
            permit = concurrency_limit().acquire() => match permit {
                Ok(p) => p,
                Err(_) => return,
            }
        };

        if !transition_pending_to_running(&bg_sub_cancel, &bg_status_ref) {
            finalize_cancelled_queued(
                &bg_core,
                &bg_parent_id,
                &bg_sub_id,
                &bg_task,
                &bg_status_ref,
                &bg_notify_tx,
            );
            return;
        }

        let started = std::time::Instant::now();
        let engine = crate::agent::engine::AgentEngine::new(bg_core.clone());
        let outcome = engine
            .execute_turn(&bg_sub_id, &bg_task, None, bg_sub_cancel.clone())
            .await;

        let _ = crate::agent::session::delete_events(&bg_core, &bg_sub_id).await;

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
            Err(error) => Err(error.to_string()),
        };

        // 更新状态与图谱；若运行期间已被外部置为 Interrupted，则保留终态不再覆盖。
        let final_status = if bg_sub_cancel.is_cancelled()
            && result.as_ref().is_ok_and(|r| r == "[subagent cancelled by user]")
        {
            super::registry::AgentLifecycleStatus::Interrupted
        } else {
            match &result {
                Ok(rep) => super::registry::AgentLifecycleStatus::Completed(Some(rep.clone())),
                Err(e) => super::registry::AgentLifecycleStatus::Errored(e.clone()),
            }
        };
        if let Ok(mut st) = bg_status_ref.write() {
            if !st.is_final() {
                *st = final_status;
            }
        }

        let report_preview = result.as_ref().ok().map(|r| r.chars().take(400).collect::<String>());
        if let Some(updated_node) = super::graph::global_graph_store().set_node_status(
            &bg_parent_id,
            &bg_sub_id,
            super::graph::ThreadSpawnEdgeStatus::Closed,
            report_preview.clone(),
        ) {
            bg_core.emit(crate::events::NovaEvent::AgentGraphUpdated {
                conversation_id: bg_parent_id.clone(),
                node: serde_json::to_value(&updated_node).unwrap_or_default(),
            });
        }

        // 最终交付成果自动写入协作看板 #research 频道
        if let Ok(ref report) = result {
            if let Ok(post) = super::board::global_message_board().post_message(
                &bg_parent_id,
                "research",
                None,
                &bg_sub_id,
                &bg_nickname,
                &bg_role,
                report,
            ) {
                bg_core.emit(crate::events::NovaEvent::BoardPostCreated {
                    conversation_id: bg_parent_id.clone(),
                    post: serde_json::to_value(&post).unwrap_or_default(),
                });
            }
        }

        emit_status_detail(
            &bg_core,
            &bg_parent_id,
            &bg_sub_id,
            if result.is_ok() { "done" } else { "error" },
            &bg_task,
            result.as_ref().err().cloned(),
            result.as_ref().ok().map(|r| r.chars().take(400).collect::<String>()),
            started.elapsed(),
        );

        bg_notify_tx.send_modify(|val| *val = val.saturating_add(1));
    });

    let reported_status = status_ref
        .read()
        .map(|s| s.to_string_repr())
        .unwrap_or("pending_init");

    Ok(serde_json::json!({
        "agent_id": sub_id,
        "nickname": effective_nickname,
        "role": effective_role,
        "status": reported_status,
        "message": format!(
            "Subagent '{}' ({}) spawned in background (initial status: {}). Use send_message to post directives, wait_agent to await completion, or list_agents to inspect cluster status.",
            effective_nickname, effective_role, reported_status
        )
    }))
}

pub async fn run(
    core: &Core,
    parent_conversation_id: &str,
    task: &str,
    role: Option<&str>,
    nickname: Option<&str>,
) -> Result<String, String> {
    let _permit = concurrency_limit()
        .acquire()
        .await
        .map_err(|e| format!("subagent semaphore closed: {}", e))?;

    let sub_id = derive_sub_id(parent_conversation_id);
    let now = chrono::Utc::now().timestamp_millis();
    let effective_role = role.unwrap_or("researcher");
    let effective_nickname = nickname.unwrap_or("ResearchSubagent");

    let initial_node = super::graph::AgentNode {
        id: sub_id.clone(),
        parent_id: Some(parent_conversation_id.to_string()),
        role: effective_role.to_string(),
        nickname: effective_nickname.to_string(),
        task: task.to_string(),
        status: super::graph::ThreadSpawnEdgeStatus::Open,
        created_at: now,
        updated_at: now,
        result_preview: None,
    };
    super::graph::global_graph_store().upsert_node(parent_conversation_id, initial_node.clone());
    core.emit(crate::events::NovaEvent::AgentGraphUpdated {
        conversation_id: parent_conversation_id.to_string(),
        node: serde_json::to_value(&initial_node).unwrap_or_default(),
    });

    emit_status(core, parent_conversation_id, &sub_id, "start", task, None);

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

    // 注册至全局智能体注册表
    let status_ref = std::sync::Arc::new(std::sync::RwLock::new(
        super::registry::AgentLifecycleStatus::Running,
    ));
    let (notify_tx, notify_rx) = tokio::sync::watch::channel(0);
    let mailbox = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::VecDeque::new()));

    let handle = std::sync::Arc::new(super::registry::AgentHandle {
        id: sub_id.clone(),
        parent_id: parent_conversation_id.to_string(),
        role: effective_role.to_string(),
        nickname: effective_nickname.to_string(),
        task: task.to_string(),
        status: status_ref.clone(),
        cancel_token: sub_cancel.clone(),
        mailbox,
        mailbox_notify: notify_tx,
        mailbox_rx: notify_rx,
        created_at: now,
    });
    super::registry::global_agent_registry().register(handle);

    // 直接复用 AgentEngine 认知引擎执行 ReAct 闭环
    let engine = crate::agent::engine::AgentEngine::new(core.clone());
    let outcome = engine.execute_turn(&sub_id, task, None, sub_cancel.clone()).await;
    sub_cancel.cancel();
    let _ = join_handle.await;

    // 清理子会话存储的临时事件，保持持久化存储轻量纯净
    let _ = crate::agent::session::delete_events(core, &sub_id).await;

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
        Err(error) => Err(error.to_string()),
    };

    // 同步更新全局生命周期状态
    let final_status = match &result {
        Ok(rep) => super::registry::AgentLifecycleStatus::Completed(Some(rep.clone())),
        Err(e) => super::registry::AgentLifecycleStatus::Errored(e.clone()),
    };
    if let Ok(mut st) = status_ref.write() {
        *st = final_status;
    }

    let report_preview = result.as_ref().ok().map(|r| r.chars().take(400).collect::<String>());
    if let Some(updated_node) = super::graph::global_graph_store().set_node_status(
        parent_conversation_id,
        &sub_id,
        super::graph::ThreadSpawnEdgeStatus::Closed,
        report_preview.clone(),
    ) {
        core.emit(crate::events::NovaEvent::AgentGraphUpdated {
            conversation_id: parent_conversation_id.to_string(),
            node: serde_json::to_value(&updated_node).unwrap_or_default(),
        });
    }

    if let Ok(ref report) = result {
        if let Ok(post) = super::board::global_message_board().post_message(
            parent_conversation_id,
            "research",
            None,
            &sub_id,
            effective_nickname,
            effective_role,
            report,
        ) {
            core.emit(crate::events::NovaEvent::BoardPostCreated {
                conversation_id: parent_conversation_id.to_string(),
                post: serde_json::to_value(&post).unwrap_or_default(),
            });
        }
    }

    emit_status_detail(
        core,
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
    core: &Core,
    parent: &str,
    sub: &str,
    phase: &str,
    task: &str,
    detail: Option<String>,
    report_preview: Option<String>,
    elapsed: std::time::Duration,
) {
    core.emit(crate::events::NovaEvent::SubagentStatus {
        parent_conversation_id: parent.to_string(),
        sub_id: sub.to_string(),
        phase: phase.to_string(),
        task: task.to_string(),
        detail,
        report_preview,
        elapsed_ms: Some(elapsed.as_millis() as u64),
    });
}

/// 供 system_prompt 分支调用：子代理系统提示词。
pub fn system_prompt() -> String {
    subagent_system_prompt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, RwLock};
    use tokio::sync::{watch, Mutex};
    use tokio_util::sync::CancellationToken;

    fn dummy_handle(id: &str, parent_id: &str, status: super::super::registry::AgentLifecycleStatus) -> Arc<super::super::registry::AgentHandle> {
        let (tx, rx) = watch::channel(0u64);
        Arc::new(super::super::registry::AgentHandle {
            id: id.to_string(),
            parent_id: parent_id.to_string(),
            role: "researcher".to_string(),
            nickname: id.to_string(),
            task: "task".to_string(),
            status: Arc::new(RwLock::new(status)),
            cancel_token: CancellationToken::new(),
            mailbox: Arc::new(Mutex::new(VecDeque::new())),
            mailbox_notify: tx,
            mailbox_rx: rx,
            created_at: 0,
        })
    }

    #[test]
    fn queued_cancel_never_transitions_to_running() {
        let cancel = CancellationToken::new();
        let status = RwLock::new(super::super::registry::AgentLifecycleStatus::PendingInit);
        cancel.cancel();
        assert!(!transition_pending_to_running(&cancel, &status));
        assert_eq!(
            *status.read().unwrap(),
            super::super::registry::AgentLifecycleStatus::Interrupted
        );

        let already_interrupted = RwLock::new(super::super::registry::AgentLifecycleStatus::Interrupted);
        let fresh_cancel = CancellationToken::new();
        assert!(!transition_pending_to_running(&fresh_cancel, &already_interrupted));
        assert_eq!(
            *already_interrupted.read().unwrap(),
            super::super::registry::AgentLifecycleStatus::Interrupted
        );
    }

    #[test]
    fn per_conversation_limit_caps_active_subagents_at_eight() {
        let registry = super::super::registry::AgentRegistry::new();
        for i in 0..MAX_SUBAGENTS_PER_CONVERSATION {
            let h = dummy_handle(
                &format!("conv:sub:{i}"),
                "conv",
                super::super::registry::AgentLifecycleStatus::PendingInit,
            );
            assert!(registry.try_register(h, MAX_SUBAGENTS_PER_CONVERSATION).is_ok());
        }
        // 第 9 个必须被原子拒绝
        let ninth = dummy_handle(
            "conv:sub:ninth",
            "conv",
            super::super::registry::AgentLifecycleStatus::PendingInit,
        );
        assert_eq!(
            registry.try_register(ninth, MAX_SUBAGENTS_PER_CONVERSATION),
            Err(MAX_SUBAGENTS_PER_CONVERSATION)
        );

        // 若其中 1 个变为终态 Completed，则允许继续补位注册
        if let Some(first) = registry.find("conv", "conv:sub:0") {
            *first.status.write().unwrap() =
                super::super::registry::AgentLifecycleStatus::Completed(Some("ok".into()));
        }
        let replacement = dummy_handle(
            "conv:sub:repl",
            "conv",
            super::super::registry::AgentLifecycleStatus::Running,
        );
        assert!(registry
            .try_register(replacement, MAX_SUBAGENTS_PER_CONVERSATION)
            .is_ok());
    }
}
