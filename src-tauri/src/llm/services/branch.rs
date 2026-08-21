//! 分支对话：从主会话选中文本派生的临时问答分支。
//!
//! 设计定位：轻量、纯问答、不持久化。
//! - 派生 scope id 形如 `{parent}:branch:{branchId}`，仿子代理 `:sub:` 约定；
//! - 系统提示词为专属精简版（system_prompt.rs 早退），不注入工程协议/Memory/插件段；
//! - 工具列表为空（tools/mod.rs 早退），模型无法执行任何工具；
//! - 不走 turn snapshot / hooks / 历史落库，分支历史完全由前端内存态持有，
//!   每轮请求全量上传；
//! - 流式事件经 stream_runner 统一出口改发 `branch-event`，与主会话 chat-stream 隔离。

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

use crate::llm::providers::LlmClient;
use crate::llm::types::{AgentMode, Message};

/// 分支会话 scope 标记：`{parent}:branch:{branchId}`。
pub const BRANCH_MARKER: &str = ":branch:";

/// 会话 ID 是否属于分支问答（格式 `{parent}:branch:{branchId}`）。
pub fn is_branch_conversation(conversation_id: Option<&str>) -> bool {
    conversation_id
        .map(|id| id.contains(BRANCH_MARKER))
        .unwrap_or(false)
}

/// 从分支 scope 提取父会话 ID；非分支 ID 原样返回。
pub fn parent_conversation_id(scope: &str) -> &str {
    match scope.split_once(BRANCH_MARKER) {
        Some((parent, _)) => parent,
        None => scope,
    }
}

/// 从分支 scope 提取分支 ID；非分支 ID 原样返回。
pub fn branch_id_from_scope(scope: &str) -> &str {
    match scope.split_once(BRANCH_MARKER) {
        Some((_, branch)) => branch,
        None => scope,
    }
}

fn make_scope(parent_conversation_id: &str, branch_id: &str) -> String {
    format!(
        "{}{}{}",
        parent_conversation_id, BRANCH_MARKER, branch_id
    )
}

/// 分支专属系统提示词：纯问答，无工具、无工作区、无工程协议。
/// 所有分支共享同一份前缀 → Anthropic prompt cache 自动命中。
pub fn system_prompt() -> String {
    r#"You are a focused Q&A assistant in a chat sidebar. The user selected a passage from an AI reply and opened this branch to ask follow-up questions about it.

Rules:
- Answer directly and concisely; match the user's language (Chinese by default).
- Stay anchored to the quoted passage: explain terms, unpack reasoning, give short concrete examples when helpful.
- You have no tools and no access to files, shell, or the main conversation beyond what the user provides here. Do not claim otherwise.
- Prefer a few clear sentences or a short list over long essays."#
        .to_string()
}

/// 运行中的分支 scope 集合：防止同一分支并发两轮（前端按钮禁用是体验层兜底）。
fn running_scopes() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 向前端推送分支生命周期状态；流式增量由 stream_runner 的 branch-event 包装转发。
fn emit_status(app: &AppHandle, scope: &str, phase: &str, detail: Option<String>) {
    let _ = app.emit(
        "branch-event",
        serde_json::json!({
            "kind": "status",
            "parentConversationId": parent_conversation_id(scope),
            "branchId": branch_id_from_scope(scope),
            "phase": phase,
            "detail": detail,
        }),
    );
}

/// 发送一轮分支问答：单次流式补全，无工具、无快照、无落库。
/// messages 为前端内存态持有的完整分支历史（首轮首条用户消息含引用原文）。
#[tauri::command]
pub async fn send_branch_message(
    app: AppHandle,
    parent_conversation_id: String,
    branch_id: String,
    messages: Vec<Message>,
) -> Result<(), String> {
    let parent = parent_conversation_id.trim().to_string();
    let branch = branch_id.trim().to_string();
    if parent.is_empty() || branch.is_empty() {
        return Err("send_branch_message requires parent_conversation_id and branch_id".to_string());
    }
    if messages.is_empty() {
        return Err("send_branch_message requires at least one message".to_string());
    }
    let scope = make_scope(&parent, &branch);

    {
        let mut running = running_scopes().lock().unwrap_or_else(|e| e.into_inner());
        if !running.insert(scope.clone()) {
            return Err("该分支已有正在进行的回复，请等待其完成或先停止。".to_string());
        }
    }

    crate::llm::cancellation::begin_turn(Some(&scope));
    emit_status(&app, &scope, "start", None);

    let result: Result<(), String> = async {
        let mut client = LlmClient::new(&app)?;
        client
            .send_request(&app, &messages, AgentMode::Agent, Some(&scope))
            .await
            .map_err(|e| e.message)?;
        Ok(())
    }
    .await;

    crate::llm::cancellation::finish_turn(Some(&scope));
    {
        let mut running = running_scopes().lock().unwrap_or_else(|e| e.into_inner());
        running.remove(&scope);
    }

    match &result {
        Ok(()) => emit_status(&app, &scope, "done", None),
        Err(err) => emit_status(&app, &scope, "error", Some(err.clone())),
    }
    result
}

/// 取消进行中的分支问答；返回是否命中运行中的分支轮次。
#[tauri::command]
pub async fn cancel_branch_message(
    parent_conversation_id: String,
    branch_id: String,
) -> Result<bool, String> {
    let scope = make_scope(parent_conversation_id.trim(), branch_id.trim());
    Ok(crate::llm::cancellation::request_cancel(Some(&scope)))
}
