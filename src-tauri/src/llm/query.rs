use tauri::{AppHandle, Emitter};

use std::collections::HashSet;

use crate::llm::providers::LlmProvider;
use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::services::compact;
use crate::llm::types::{AgentMode, Content, ContentBlock, Message, Role};
use crate::llm::utils::context_assembler::{self, AssembleOptions};
use crate::llm::utils::error_event::emit_backend_error;

mod state_machine;

use state_machine::TurnOutcome;

const SESSION_RAG_CONTEXT_MARKER: &str = "[Session RAG Context]";
const SESSION_RAG_SEARCH_LIMIT: usize = 5;
const MCP_SERVER_CONTEXT_MARKER: &str = "[MCP Server Catalog]";
const RESPONSE_RESERVE_TOKENS: u32 = 8_000;

fn clamp_i64_to_u32(value: i64) -> u32 {
    if value <= 0 {
        0
    } else if value >= u32::MAX as i64 {
        u32::MAX
    } else {
        value as u32
    }
}

fn emit_token_usage_event(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    source: &str,
) {
    let total_tokens = match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => input.checked_add(output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    };

    let payload = serde_json::json!({
        "inputTokens": input_tokens,
        "outputTokens": output_tokens,
        "totalTokens": total_tokens,
        "source": source,
    });

    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "token-usage".into(),
            text: Some(payload.to_string()),
            tool_use_id: None,
            tool_use_name: None,
            tool_use_input: None,
            tool_result: None,
            token_usage: total_tokens,
            stop_reason: None,
            turn_state: Some("usage".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}

fn emit_context_compact_event(
    app: &AppHandle,
    conversation_id: Option<&str>,
    level: &str,
    reason: &str,
    before_tokens: u32,
    after_tokens: u32,
) {
    let saved_tokens = before_tokens.saturating_sub(after_tokens);
    if saved_tokens == 0 {
        return;
    }
    eprintln!(
        "[compact] applied level={} before={} after={} saved={} reason={}",
        level, before_tokens, after_tokens, saved_tokens, reason
    );
    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "context-compact".into(),
            text: Some(
                serde_json::json!({
                    "level": level,
                    "reason": reason,
                    "beforeTokens": before_tokens,
                    "afterTokens": after_tokens,
                    "savedTokens": saved_tokens,
                })
                .to_string(),
            ),
            tool_use_id: None,
            tool_use_name: None,
            tool_use_input: None,
            tool_result: None,
            token_usage: None,
            stop_reason: None,
            turn_state: Some("context_compacted".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}

fn emit_context_usage_event(
    app: &AppHandle,
    conversation_id: Option<&str>,
    used_tokens: u32,
    window_tokens: u32,
    source: &str,
) {
    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "context-usage".into(),
            text: Some(
                serde_json::json!({
                    "usedTokens": used_tokens,
                    "windowTokens": window_tokens,
                    "responseReserveTokens": RESPONSE_RESERVE_TOKENS,
                    "source": source,
                })
                .to_string(),
            ),
            tool_use_id: None,
            tool_use_name: None,
            tool_use_input: None,
            tool_result: None,
            token_usage: None,
            stop_reason: None,
            turn_state: Some("usage".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
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
        // Strip the RAG notice lines appended by the frontend so they don't
        // pollute BM25 query terms. The notice starts with the marker line and
        // continues to the end of the text.
        let clean = text
            .lines()
            .take_while(|line| !line.trim().starts_with("已上传文件（可在会话RAG中检索）："))
            .collect::<Vec<_>>()
            .join("\n");
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn latest_user_raw_text(messages: &[Message]) -> Option<String> {
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

fn truncate_chars(input: &str, limit: usize) -> String {
    let mut chars = input.chars();
    let snippet: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{}...", snippet)
    } else {
        snippet
    }
}

fn extract_uploaded_document_names(query: &str) -> Vec<String> {
    query
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("已上传文件（可在会话RAG中检索）：")
                .map(str::trim)
        })
        .map(|raw| {
            raw.split("，")
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn build_direct_attachment_context(
    app: &AppHandle,
    conversation_id: &str,
    source_names: &[String],
) -> Result<Vec<String>, String> {
    if source_names.is_empty() {
        return Ok(Vec::new());
    }

    let documents = crate::command::rag::rag_list_conversation_documents(
        app.clone(),
        conversation_id.to_string(),
    )?;

    let mut lines = Vec::new();

    for source_name in source_names {

        let Some(doc) = documents.iter().find(|doc| doc.source_name == *source_name) else {
            continue;
        };

        let Some(content) = crate::command::rag::rag_read_document(app.clone(), doc.id.clone())?
        else {
            continue;
        };

        lines.push(format!(
            "Attached document: {} (id={}, chars={})",
            content.source_name, content.id, content.content_chars
        ));
        lines.push(content.content.clone());
    }

    Ok(lines)
}

fn build_session_rag_context_message(
    app: &AppHandle,
    conversation_id: Option<&str>,
    query: &str,
    raw_text: &str,
) -> Result<Option<Message>, String> {
    let Some(scope_id) = conversation_id
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };

    let query_text = query.trim();
    if query_text.chars().count() < 2 {
        return Ok(None);
    }

    // 用原始文本（含文件名行）提取附件名，用清洁 query 做 BM25 检索。
    let attached_source_names = extract_uploaded_document_names(raw_text);
    let direct_attachment_lines =
        build_direct_attachment_context(app, scope_id, &attached_source_names)?;

    let hits = crate::command::rag::rag_search_conversation_documents(
        app.clone(),
        scope_id.to_string(),
        query_text.to_string(),
        Some(SESSION_RAG_SEARCH_LIMIT),
    )?;

    if hits.is_empty() {
        if direct_attachment_lines.is_empty() {
            return Ok(None);
        }
    }

    let mut context_lines = vec![
		format!("{} Query: {}", SESSION_RAG_CONTEXT_MARKER, query_text),
		"Use the retrieved snippets below as supporting context. If they conflict with current repository reality or explicit user instructions, prioritize repository reality and user intent.".to_string(),
	];

    if !direct_attachment_lines.is_empty() {
        context_lines.push("Directly attached documents for this turn:".to_string());
        context_lines.extend(direct_attachment_lines);
    }

    if !hits.is_empty() {
        context_lines.push("Retrieved snippets:".to_string());
    }
    for (idx, hit) in hits.iter().enumerate() {
        context_lines.push(format!(
            "{}. {} (score={}, id={})",
            idx + 1,
            hit.source_name,
            hit.score,
            hit.id
        ));
        context_lines.push(format!("   snippet: {}", hit.snippet));
    }

    Ok(Some(Message {
        role: Role::User,
        content: Content::Text(context_lines.join("\n")),
    }))
}

async fn build_mcp_server_context_message(app: &AppHandle) -> Option<Message> {
    let statuses = crate::llm::services::mcp_tools::connected_server_catalog(app).await;
    if statuses.is_empty() {
        return None;
    }

    let mut lines = vec![
		MCP_SERVER_CONTEXT_MARKER.to_string(),
		"Connected MCP servers are available. Do not assume their internal tools up front.".to_string(),
		"Use `mcp_auth` with `action=\"list_tools\"` to inspect a server before calling one of its tools.".to_string(),
		"Use `mcp_auth` with `action=\"call_tool\"` to invoke a specific MCP tool after inspection.".to_string(),
	];

    for status in statuses {
        lines.push(format!(
            "- {} (type={}, tools={})",
            status.name, status.r#type, status.tool_count
        ));
    }

    Some(Message {
        role: Role::User,
        content: Content::Text(lines.join("\n")),
    })
}

fn is_session_start_turn(messages: &[Message]) -> bool {
    let assistant_count = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    let user_count = messages.iter().filter(|m| m.role == Role::User).count();

    assistant_count == 0 && user_count <= 1
}

fn apply_post_compact_hook(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &mut Vec<Message>,
) {
    let post_compact_hook =
        crate::llm::services::hooks::run_post_compact_hooks(app, conversation_id);
    if !post_compact_hook.additional_messages.is_empty() {
        messages.extend(post_compact_hook.additional_messages);
    }
}

// 从消息列表中移除每轮动态注入的上下文消息（RAG、MCP catalog、会话恢复、全局记忆、所有 hook 注入）。
// 保存快照前调用，确保快照只包含真实对话内容。
fn strip_injected_context(messages: &mut Vec<Message>) {
    const MARKERS: &[&str] = &[
        SESSION_RAG_CONTEXT_MARKER,
        MCP_SERVER_CONTEXT_MARKER,
        "[Session Restore Context]",
        "[Global Memory]",
        // lifecycle hooks — 每轮动态注入，不应固化进 snapshot
        "[SessionStart]",
        "[UserPromptSubmit]",
        "[PreCompact]",
        "[PostCompact]",
        "[SubagentStart]",
        "[SubagentStop]",
        // tool flow hooks
        "[PreToolUse]",
        "[PostToolUse]",
        "[PostToolUseFailure]",
        // stop hooks
        "[StopHookContext]",
    ];
    messages.retain(|m| {
        let text = text_from_content(&m.content);
        !MARKERS.iter().any(|marker| text.starts_with(marker))
    });
}

// 入口函数：发送用户聊天消息，驱动整轮 LLM 编排。
// 它负责：
// 1) 在真正请求模型前准备上下文（hooks / session restore / compact / session RAG）
// 2) 循环调用 provider，并把 provider 返回的新消息与 tool_result 回灌到 current_messages
// 3) 根据 needs_user_input / cancelled / prevent_continuation / has_tool_result 决定是否续跑
// 4) 在正常结束路径统一执行 session_end_hooks 并发送 stop 事件
// 5) 在 provider 错误路径执行 error_hooks，发送 stop(error) 后直接返回 Err
//
// send_chat_message
//     │
//     ├─ 1. 回合前准备
//     │       ├─ run_user_prompt_submit_hooks      → 追加提示提交上下文
//     │       └─ (首轮) run_session_start_hooks    → 追加会话开始上下文
//     │
//     ├─ 2. 上下文构建
//     │       ├─ context_assembler                 → 注入会话恢复上下文
//     │       ├─ run_pre_compact_hooks             → 压缩前上下文扩展
//     │       ├─ compact                           → 压缩历史消息 / 大型 tool_result
//     │       └─ session rag retrieval             → 仅按当前会话文档检索并注入上下文
//     │
//     ├─ 3. 主循环 loop
//     │       ├─ 取消检查                          → cancelled → break
//     │       ├─ 应用已提交的权限决策 / 维持审批状态
//     │       ├─ provider.send_request (流式)
//     │       │       └─ 错误: run_error_hooks + emit stop(error) + return Err
//     │       ├─ provider 报告 cancelled           → break
//     │       ├─ 合并新消息到 current_messages
//     │       ├─ needs_user_input                  → break
//     │       ├─ provider_result.prevent_continuation
//     │       │       └─ stop_hook_prevented       → break
//     │       ├─ has_tool_result                   → continue (下一轮，等待模型消费 tool_result)
//     │       └─ !has_tool_result
//     │               ├─ run_stop_hooks
//     │               │       ├─ prevent_continuation → break
//     │               │       └─ added_context → current_messages.extend → continue
//     │               └─ 正常结束                 → completed → break
//     │
//     └─ 4. 回合收尾（正常路径）
//             ├─ run_session_end_hooks             → 可覆盖 stop_reason
//             └─ emit stop                         → return Ok
pub async fn send_chat_message(
    app: AppHandle,
    conversation_id: Option<String>,
    messages: Vec<Message>,
    agent_mode: AgentMode,
) -> Result<(), String> {
    let rag_query = latest_user_query_text(&messages);
    let rag_raw_text = latest_user_raw_text(&messages);
    let session_start_turn = is_session_start_turn(&messages);
    // 记录前端传入消息数量，用于之后从 turn_messages 中定位"本轮新消息"起始位置。
    let frontend_msg_count = messages.len();
    let mut turn_messages = messages;

    let prompt_submit_hook =
        crate::llm::services::hooks::run_user_prompt_submit_hooks(&app, conversation_id.as_deref());
    if !prompt_submit_hook.additional_messages.is_empty() {
        turn_messages.extend(prompt_submit_hook.additional_messages);
    }

    if session_start_turn {
        let session_start_hook =
            crate::llm::services::hooks::run_session_start_hooks(&app, conversation_id.as_deref());
        if !session_start_hook.additional_messages.is_empty() {
            turn_messages.extend(session_start_hook.additional_messages);
        }
    }

    // 尝试加载上一轮保存的完整模型上下文快照（含 tool_use / tool_result blocks）。
    // - 有快照：用快照恢复历史，只追加本轮新增输入和 hooks 上下文。
    // - 首轮无快照：允许用前端传入的当前用户消息启动会话。
    // - 非首轮无快照：视为后端状态缺失，直接报错，避免用前端 UI 历史兜底。
    let working_messages = if let Some(conv_id) = conversation_id.as_deref() {
        match crate::llm::history::load_turn_snapshot(&app, conv_id).await {
            Ok(Some(mut snap)) => {
                // 剥离每轮动态注入的上下文，后续会按当前状态重新注入。
                strip_injected_context(&mut snap);
                // 前端消息只用来定位本轮新增输入；历史必须来自 snapshot。
                // hooks 已追加到 turn_messages 尾部，因此从最新用户消息开始整体追加。
                let new_start = frontend_msg_count.saturating_sub(1);
                snap.extend_from_slice(&turn_messages[new_start..]);
                snap
            }
            Ok(None) if session_start_turn => turn_messages,
            Ok(None) => {
                return Err(format!(
                    "会话 {} 缺少 turn snapshot，且不是首轮请求，拒绝使用前端历史兜底",
                    conv_id
                ));
            }
            Err(e) => {
                return Err(format!("加载会话 {} 的 turn snapshot 失败: {}", conv_id, e));
            }
        }
    } else {
        return Err("send_chat_message requires conversation_id".to_string());
    };
    // println!("{:?}", working_messages);

    // 1. 每轮都先组装请求前的全局记忆。
    // 正常 agent 流只信任新设计下的 turn snapshot：
    // 首轮用当前输入启动，非首轮缺 snapshot 已在前面报错；
    // 因此这里不注入 session_restore，避免用摘要恢复污染模型上下文。
    let mut assembled_messages = context_assembler::assemble_messages_for_turn(
        &app,
        conversation_id.as_deref(),
        &working_messages,
        AssembleOptions {
            include_session_restore: false,
            include_env_contexts: false,
        },
    )
    .await;

    // 压缩前 hook：当前实现只读取 settings.hook_env["NOVA_PRE_COMPACT_HOOK_CONTEXT"]。
    // 如果配置存在，会追加一条 "[PreCompact] ..." 用户消息。
    // 放在 compact 前，是为了让这条临时上下文也参与 token 估算和压缩决策。
    let pre_compact_hook =
        crate::llm::services::hooks::run_pre_compact_hooks(&app, conversation_id.as_deref());
    if !pre_compact_hook.additional_messages.is_empty() {
        assembled_messages.extend(pre_compact_hook.additional_messages);
    }

    // 根据当前模型上下文窗口选择压缩策略：
    // - none：不压缩；
    // - micro：本地截断/瘦身较大的 tool_result；
    // - full：先 micro，再用模型总结旧上下文并保留最近消息。
    // 返回的 messages 会成为本轮真正继续往下传的 current_messages。
    let compact_outcome = compact::compact_messages_for_turn_with_report(
        &app,
        conversation_id.as_deref(),
        &assembled_messages,
    )
    .await;

    // 只有真的发生 compact 时才跑 post compact hook。
    // 当前 post hook 同样是配置文本注入：settings.hook_env["NOVA_POST_COMPACT_HOOK_CONTEXT"]。
    // compact 通知只用于前端展示本轮节省了多少上下文，不改变历史来源。
    let did_compact = compact_outcome.did_compact();
    let mut current_messages = compact_outcome.messages;
    if did_compact {
        apply_post_compact_hook(&app, conversation_id.as_deref(), &mut current_messages);
        let after_tokens =
            clamp_i64_to_u32(compact::estimate_tokens_for_messages(&current_messages));
        emit_context_compact_event(
            &app,
            conversation_id.as_deref(),
            compact_outcome.level,
            "自动压缩历史上下文，减少发送给模型的背景信息体积。",
            clamp_i64_to_u32(compact_outcome.estimated_tokens),
            after_tokens,
        );
    }

    // 会话 RAG 是本轮临时上下文增强：
    // 使用最新用户文本检索当前 conversation 绑定的文档，并把命中的片段追加到 current_messages。
    // 检索失败只发 backend-error，不中断主对话；保存 snapshot 前会剥掉该类临时注入。
    if let Some(query_text) = rag_query.as_deref() {
        let raw_text = rag_raw_text.as_deref().unwrap_or(query_text);
        match build_session_rag_context_message(&app, conversation_id.as_deref(), query_text, raw_text) {
            Ok(Some(rag_context)) => current_messages.push(rag_context),
            Ok(None) => {}
            Err(e) => {
                emit_backend_error(
                    &app,
                    "rag.session_search",
                    format!("会话知识库检索失败，本轮将跳过 RAG 上下文增强：{}", e),
                    Some("build_session_rag_context_message"),
                );
            }
        }
    }

    // MCP catalog 也是本轮临时上下文：
    // 告诉模型当前连接了哪些 MCP server，但不提前展开具体工具。
    // 模型后续需要时再通过 mcp_auth/list_tools/call_tool 走正式工具流。
    if let Some(mcp_context) = build_mcp_server_context_message(&app).await {
        current_messages.push(mcp_context);
    }
    // println!("current_messages:{:?}", current_messages);

    // 2. 根据设置选择模型提供方（Anthropic/OpenAI）。
    // Provider 实例封装了底层调用细节。
    let provider = LlmProvider::new(&app);

    // 3. 主循环：调用 provider.send_request（流式），并根据 tool 执行情况决定是否继续下一步。
    //    - 如果发生工具调用，结果会被“注入”到 current_messages 继续下一轮。
    //    - 如果 provider 返回 needs_user_input / 无工具结果，则结束。
    let mut has_attempted_reactive_compact = false;
    let mut final_outcome = loop {
        // 若收到取消请求，则立即以 cancelled 结束。
        if crate::llm::cancellation::is_cancelled(conversation_id.as_deref()) {
            break TurnOutcome::cancelled();
        }

        // 每次请求 provider 前重新读取当前模型配置，拿到该模型的上下文窗口大小。
        // 模型可能在设置中切换，因此这里不复用回合开始时的窗口值。
        let model = crate::command::settings::get_settings(app.clone())
            .active_provider_profile()
            .model;

        let window_tokens =
            crate::llm::utils::model_context::get_context_window_tokens(&model) as i64;
        // 工具结果上下文编辑：专门处理较早、较大的 tool_use/tool_result 对。
        // 它不同于前面的整体 compact；这里在主循环每次 provider 请求前执行，
        // 用于防止多轮工具调用后旧工具输出持续占满上下文窗口。
        let context_editing =
            compact::apply_tool_result_context_editing(&current_messages, window_tokens);
        if context_editing.applied {
            // 仅当真的清理了工具结果时通知前端，并用编辑后的 messages 继续本轮 loop。
            emit_context_compact_event(
                &app,
                conversation_id.as_deref(),
                "tool_result",
                &format!(
                    "清理了 {} 组较早的工具结果，避免大型工具输出占满上下文。",
                    context_editing.cleared_tool_pairs
                ),
                clamp_i64_to_u32(context_editing.original_estimated_tokens),
                clamp_i64_to_u32(context_editing.edited_estimated_tokens),
            );
            current_messages = context_editing.messages;
        }

        // println!(
        //     "model: {}, window_tokens: {}, context_editing_applied: {}, cleared_tool_pairs: {}, original_tokens: {}, edited_tokens: {}",
        //     &model,
        //     window_tokens,
        //     context_editing.applied,
        //     context_editing.cleared_tool_pairs,
        //     context_editing.original_estimated_tokens,
        //     context_editing.edited_estimated_tokens
        // );

        // 请求 provider 前先估算当前 prompt 占用，并通知前端更新 context window UI。
        // 这是本地估算值，不参与模型调用；provider 返回真实 usage 后会再用 actual 数据校正。

        let request_input_estimate =
            clamp_i64_to_u32(compact::estimate_tokens_for_messages(&current_messages));
        emit_context_usage_event(
            &app,
            conversation_id.as_deref(),
            request_input_estimate,
            window_tokens as u32,
            "estimated",
        );

        // 发起 provider 请求并等待结果。
        let provider_result = match provider
            .send_request(
                &app,
                &current_messages,
                agent_mode,
                conversation_id.as_deref(),
            )
            .await
        {
            // 请求成功时拿到结果对象。
            Ok(v) => v,
            Err(provider_err) => {
                let e = provider_err.message.clone();
                if !has_attempted_reactive_compact && compact::is_prompt_too_long_error(&e) {
                    if let Some(recovered_messages) = compact::reactive_compact_messages_for_retry(
                        &app,
                        conversation_id.as_deref(),
                        &current_messages,
                    )
                    .await
                    {
                        let before_tokens = clamp_i64_to_u32(
                            compact::estimate_tokens_for_messages(&current_messages),
                        );
                        let after_tokens = clamp_i64_to_u32(compact::estimate_tokens_for_messages(
                            &recovered_messages,
                        ));
                        current_messages = recovered_messages;
                        apply_post_compact_hook(
                            &app,
                            conversation_id.as_deref(),
                            &mut current_messages,
                        );
                        emit_context_compact_event(
                            &app,
                            conversation_id.as_deref(),
                            "reactive",
                            "模型提示上下文过长，已自动压缩后重试。",
                            before_tokens,
                            after_tokens,
                        );
                        has_attempted_reactive_compact = true;
                        continue;
                    }
                }

                // 流中断前已有部分输出时，保存 partial snapshot，避免下轮上下文丢失。
                if !provider_err.partial_messages.is_empty() {
                    if let Some(conv_id) = conversation_id.as_deref() {
                        let mut snapshot = current_messages.clone();
                        snapshot.extend(provider_err.partial_messages);
                        strip_injected_context(&mut snapshot);
                        // 错误路径的 snapshot 保存是 best-effort，失败不阻断错误返回。
                        let _ = crate::llm::history::save_turn_snapshot(&app, conv_id, &snapshot).await;
                    }
                }

                let error_hook = crate::llm::services::hooks::run_error_hooks(
                    &app,
                    &e,
                    conversation_id.as_deref(),
                );
                let error_text = error_hook.override_error.unwrap_or_else(|| e.clone());
                // 出错直接通知前端 stop(error) 并返回错误。
                // 同时上报后端错误事件用于统一监控。
                emit_backend_error(
                    &app,
                    "llm.query_engine",
                    error_text.clone(),
                    Some("provider.send_request"),
                );
                // 通知前端当前回合以错误状态结束。
                app.emit(
                    "chat-stream",
                    ChatMessageEvent {
                        // 事件类型为 stop。
                        r#type: "stop".into(),
                        // 把错误文本透传给前端。
                        text: Some(error_text.clone()),
                        // 以下字段在 stop 事件中均为空。
                        tool_use_id: None,
                        tool_use_name: None,
                        tool_use_input: None,
                        tool_result: None,
                        token_usage: None,
                        // 停止原因标记为 provider_error。
                        stop_reason: Some("provider_error".into()),
                        // 回合状态标记为 error。
                        turn_state: Some("error".into()),
                        // 透传会话 ID，便于前端路由到正确会话。
                        conversation_id: conversation_id.clone(),
                    },
                )
                // 忽略 emit 错误，保证主错误路径返回。
                .ok();
                // 将 provider 错误返回给上层调用方。
                return Err(error_text);
            }
        };

        // provider 主动报告取消时，统一收敛为 cancelled。
        if provider_result.stop_reason.as_deref() == Some("cancelled") {
            // 1. 保留模型说到一半的半截话，避免上下文丢失。
            current_messages.extend(provider_result.messages.clone());

            // 2. 查找并闭合这半截话里所有未完成的 tool_use，防止 API 语法校验报错。
            let existing_tool_result_ids = provider_result
                .messages
                .iter()
                .filter_map(|msg| {
                    if let Content::Blocks(blocks) = &msg.content {
                        Some(blocks)
                    } else {
                        None
                    }
                })
                .flat_map(|blocks| blocks.iter())
                .filter_map(|block| {
                    if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                        Some(tool_use_id.clone())
                    } else {
                        None
                    }
                })
                .collect::<HashSet<_>>();

            let mut user_blocks = Vec::new();
            for msg in &provider_result.messages {
                if let Content::Blocks(blocks) = &msg.content {
                    for block in blocks {
                        if let ContentBlock::ToolUse { id, .. } = block {
                            if existing_tool_result_ids.contains(id) {
                                continue;
                            }
                            user_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                is_error: false,
                                content: vec![ContentBlock::Text {
                                    text: "Interrupted by user".to_string(),
                                }],
                            });
                        }
                    }
                }
            }

            // 3. 追加中断标记，确保模型在下一轮明确知道这是被用户主动打断的。
            user_blocks.push(ContentBlock::Text {
                text: "[Request interrupted by user]".to_string(),
            });

            current_messages.push(Message {
                role: Role::User,
                content: Content::Blocks(user_blocks),
            });

            break TurnOutcome::cancelled();
        }

        let input_tokens = provider_result
            .input_tokens
            .or(Some(request_input_estimate))
            .filter(|value| *value > 0);
        let input_token_source = if provider_result.input_tokens.is_some() {
            "actual"
        } else {
            "estimated"
        };
        emit_token_usage_event(
            &app,
            conversation_id.as_deref(),
            input_tokens,
            provider_result.output_tokens,
            input_token_source,
        );

        // 若 provider 返回了实际 input_tokens，用真实值刷新上下文用量显示。
        if let Some(actual_input) = provider_result.input_tokens {
            emit_context_usage_event(
                &app,
                conversation_id.as_deref(),
                actual_input,
                window_tokens as u32,
                "actual",
            );
        }

        // 本轮 provider 输出合并到 current_messages 以支持工具环回。
        // 取出本轮新增消息。
        let new_messages = provider_result.messages;
        // 将新增消息并入上下文，供后续轮继续使用。
        current_messages.extend(new_messages.clone());

        // 判断新增消息中是否包含 tool_result 块。
        let has_tool_result = new_messages.iter().any(|m| {
            // 仅 blocks 结构里可能包含 tool_result。
            if let Content::Blocks(blocks) = &m.content {
                blocks
                    .iter()
                    // 只要有任意 ToolResult 块就判定为 true。
                    .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
            } else {
                // 非 blocks 内容不可能包含 tool_result。
                false
            }
        });

        if matches!(
            provider_result.stop_reason.as_deref(),
            Some("tool_calls" | "tool_use")
        ) && !has_tool_result
        {
            let msg = format!(
				"Provider returned stop_reason={:?} but query found no ToolResult in new_messages. new_messages={}",
				provider_result.stop_reason,
				truncate_chars(&format!("{:?}", new_messages), 4000)
			);
            emit_backend_error(
                &app,
                "llm.query.tool_call_invariant",
                msg.clone(),
                Some("provider_result"),
            );
            app.emit(
                "chat-stream",
                ChatMessageEvent {
                    r#type: "stop".into(),
                    text: Some(msg.clone()),
                    tool_use_id: None,
                    tool_use_name: None,
                    tool_use_input: None,
                    tool_result: None,
                    token_usage: None,
                    stop_reason: Some("provider_error".into()),
                    turn_state: Some("error".into()),
                    conversation_id: conversation_id.clone(),
                },
            )
            .ok();
            return Err(msg);
        }

        // 若返回需要用户输入，终止当前回合并告诉前端。
        if compact::has_needs_user_input(&new_messages) {
            break TurnOutcome::needs_user_input();
        }

        // 若 hook/provider 明确要求停止续跑，则按 stop_hook_prevented 结束。
        if provider_result.prevent_continuation {
            break TurnOutcome::stop_hook_prevented(
                provider_result
                    .stop_reason
                    // 未给停止原因时提供默认值。
                    .unwrap_or_else(|| "hook_stopped_continuation".to_string()),
            );
        }

        // 若本轮没有工具结果，说明回合结束。
        if !has_tool_result {
            // 在回合结束前执行 stop hooks。
            let stop_hook_result = crate::llm::services::hooks::run_stop_hooks(
                &app,
                &current_messages,
                conversation_id.as_deref(),
            );
            // 判断 stop hooks 是否注入了附加上下文。
            let stop_hook_added_context = !stop_hook_result.additional_messages.is_empty();
            if stop_hook_added_context {
                // 将 stop hooks 注入的上下文并入当前消息。
                current_messages.extend(stop_hook_result.additional_messages);
            }

            // stop hooks 要求阻断续跑时立即结束。
            if stop_hook_result.prevent_continuation {
                break TurnOutcome::stop_hook_prevented(
                    stop_hook_result
                        .stop_reason
                        // 缺省停止原因兜底。
                        .unwrap_or_else(|| "stop_hook_prevented".to_string()),
                );
            }

            // 仅追加了上下文但未阻断时，继续下一轮请求。
            if stop_hook_added_context {
                continue;
            }

            // 正常结束本轮，若 provider 未给 stop_reason 则使用 end_turn。
            break TurnOutcome::completed(
                provider_result
                    .stop_reason
                    .unwrap_or_else(|| "end_turn".to_string()),
            );
        }
    };

    let session_end_hook = crate::llm::services::hooks::run_session_end_hooks(
        &app,
        &final_outcome.stop_reason,
        conversation_id.as_deref(),
    );
    if let Some(hooked_reason) = session_end_hook.stop_reason {
        final_outcome.stop_reason = hooked_reason;
    }

    // 保存本轮完整消息快照（含 tool_use / tool_result blocks），供下一轮直接复用。
    // 保存前剥离动态注入上下文（RAG/MCP/session_restore/global_memory），它们每轮重新生成。
    if let Some(conv_id) = conversation_id.as_deref() {
        let mut snapshot = current_messages.clone();
        strip_injected_context(&mut snapshot);
        if let Err(e) = crate::llm::history::save_turn_snapshot(&app, conv_id, &snapshot).await {
            let error_text = format!("保存会话 {} 的 turn snapshot 失败: {}", conv_id, e);
            emit_backend_error(
                &app,
                "llm.turn_snapshot.save",
                error_text.clone(),
                Some("save_turn_snapshot"),
            );
            return Err(error_text);
        }
    }

    // 4. 业务终止：告知前端本轮结束，并携带 stop_reason/turn_state 以区分 completed/needs_user_input/error。
    // 统一发送 stop 事件，前端据此收口渲染状态。
    app.emit(
        "chat-stream",
        ChatMessageEvent {
            // stop 事件类型。
            r#type: "stop".into(),
            // 正常 stop 不携带 text 内容。
            text: None,
            // stop 事件不绑定具体工具调用。
            tool_use_id: None,
            tool_use_name: None,
            tool_use_input: None,
            tool_result: None,
            // 本事件不附加 token_usage。
            token_usage: None,
            // 透传最终停止原因。
            stop_reason: Some(final_outcome.stop_reason),
            // 透传最终回合状态字符串。
            turn_state: Some(final_outcome.turn_state.as_event_state().to_string()),
            // 透传会话 ID，便于前端路由到正确会话。
            conversation_id: conversation_id.clone(),
        },
    )
    // stop 事件投递失败不影响函数返回。
    .ok();

    // 全流程成功完成。
    Ok(())
}
