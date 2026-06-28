use tauri::{AppHandle, Emitter};
use tracing::info;

use std::collections::HashSet;

use crate::llm::providers::LlmClient;
use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::services::compact;
use crate::llm::types::{AgentMode, Content, ContentBlock, Message, Role};
use crate::llm::utils::context_assembler::{self, AssembleOptions};
use crate::llm::utils::error_event::emit_backend_error;
use crate::llm::utils::pricing::TurnCostBreakdown;

mod state_machine;

use state_machine::TurnOutcome;

fn strip_images_to_text(messages: &[Message]) -> Vec<Message> {
    const PLACEHOLDER: &str = "错误：当前模型不支持图片输入，请告知用户切换到支持图片输入的模型，或描述图片内容。";
    messages
        .iter()
        .map(|msg| {
            let content = match &msg.content {
                Content::Text(text) => Content::Text(text.clone()),
                Content::Blocks(blocks) => Content::Blocks(
                    blocks
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Image { .. } => ContentBlock::Text {
                                text: PLACEHOLDER.to_string(),
                            },
                            other => other.clone(),
                        })
                        .collect(),
                ),
            };
            Message {
                role: msg.role.clone(),
                content,
            }
        })
        .collect()
}
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

// 发送 token 用量到前端
// 作用：每次 LLM 请求完成后，把 token 用量通过事件发送给前端 UI 显示。
// 计算逻辑：total_input = input + cache_read + cache_creation; total_tokens = total_input + output
// 发送的数据：inputTokens, outputTokens, cacheReadTokens, cacheCreationTokens, totalInputTokens, totalTokens
// 前端收到后：更新"本次 X · 会话 Y"的显示。
fn emit_token_usage_event(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    cache_read_tokens: Option<u32>,
    cache_creation_tokens: Option<u32>,
    cost: Option<&TurnCostBreakdown>,
    source: &str,
) {
    // 根据 Anthropic 文档：
    // total_input = input_tokens + cache_read_tokens + cache_creation_tokens
    // total_all = total_input + output_tokens
    let total_input = input_tokens.unwrap_or(0)
        + cache_read_tokens.unwrap_or(0)
        + cache_creation_tokens.unwrap_or(0);
    let total_tokens = total_input.checked_add(output_tokens.unwrap_or(0));

    let payload = serde_json::json!({
        "inputTokens": input_tokens,
        "outputTokens": output_tokens,
        "cacheReadTokens": cache_read_tokens,
        "cacheCreationTokens": cache_creation_tokens,
        "totalInputTokens": total_input,
        "totalTokens": total_tokens,
        "cost": cost,
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
            tool_is_error: None,
            token_usage: total_tokens,
            stop_reason: None,
            turn_state: Some("usage".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}

// 对比"本地估算值"和"API 返回真实值"的差异，输出到 stderr 日志。
// 作用：调试 token 估算准确性，如果差异太大说明估算逻辑需要优化。
// 输出内容：estimatedInputTokens, actualInputTokens, inputDelta, inputDeltaPercent, toolCount
// 用途：纯开发者调试日志，不影响前端 UI。
fn emit_token_debug_event(
    app: &AppHandle,
    conversation_id: Option<&str>,
    estimate_source: &str,
    estimated_input_tokens: u32,
    actual_input_tokens: Option<u32>,
    actual_output_tokens: Option<u32>,
    tool_count: usize,
) {
    let actual_total_tokens = actual_input_tokens
        .zip(actual_output_tokens)
        .and_then(|(input, output)| input.checked_add(output));
    let input_delta =
        actual_input_tokens.map(|actual| actual as i64 - estimated_input_tokens as i64);
    let input_delta_percent = actual_input_tokens.and_then(|actual| {
        if estimated_input_tokens == 0 {
            None
        } else {
            Some(
                ((actual as f64 - estimated_input_tokens as f64) / estimated_input_tokens as f64)
                    * 100.0,
            )
        }
    });

    let payload = serde_json::json!({
        "conversationId": conversation_id,
        "estimateSource": estimate_source,
        "estimatedInputTokens": estimated_input_tokens,
        "actualInputTokens": actual_input_tokens,
        "actualOutputTokens": actual_output_tokens,
        "actualTotalTokens": actual_total_tokens,
        "inputDelta": input_delta,
        "inputDeltaPercent": input_delta_percent,
        "toolCount": tool_count,
    });

    eprintln!("[Nova token compare] {}", payload);

    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "token-debug".into(),
            text: Some(payload.to_string()),
            tool_use_id: None,
            tool_use_name: None,
            tool_use_input: None,
            tool_result: None,
            tool_is_error: None,
            token_usage: actual_total_tokens.or(actual_input_tokens),
            stop_reason: None,
            turn_state: Some("token_debug".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}

// 当上下文太长需要压缩时，发送压缩结果给前端。
// 作用：通知前端上下文压缩已执行，显示节省了多少 token。
// 参数：level（压缩级别）、reason（原因）、before_tokens/after_tokens（压缩前后 token 数）
// 逻辑：saved_tokens = before - after，如果没省到 token 就不发事件
// 前端收到后：显示"上下文已压缩，节省了 X token"的通知。
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
    info!(
        conversation_id = %conversation_id.unwrap_or("__default__"),
        level = %level,
        before_tokens,
        after_tokens,
        saved_tokens,
        reason = %reason,
        "context compact applied"
    );
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
            tool_is_error: None,
            token_usage: None,
            stop_reason: None,
            turn_state: Some("context_compacted".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}

// 更新前端的上下文进度条（"243/1.0M 个令牌"那个）。
// 作用：发送当前上下文使用量给前端，更新进度条显示。
// 发送的数据：usedTokens（已用 token）、windowTokens（窗口大小）、responseReserveTokens（预留 8000）
// 调用时机：请求前用估算值，请求后用 API 返回的真实值覆盖。
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
            tool_is_error: None,
            token_usage: None,
            stop_reason: None,
            turn_state: Some("usage".into()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}

// 从消息内容中提取纯文本
// 作用：把 Content::Text 或 Content::Blocks 统一转成纯文本字符串。
// 处理逻辑：Content::Text 直接 trim 返回；Content::Blocks 只取 Text 类块，跳过图片/工具调用，用 \n 拼接
// 用途：后续 strip_injected_context 需要扫描文本内容来移除动态注入的上下文（如 RAG、MCP 目录）。
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

// 截断字符串到指定长度
// 作用：限制日志/错误消息的长度，避免输出过长。
// 处理逻辑：取前 limit 个字符，如果超出则加 "..." 后缀。
// 用途：日志输出、错误消息、调试信息等场景。
fn truncate_chars(input: &str, limit: usize) -> String {
    let mut chars = input.chars();
    let snippet: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{}...", snippet)
    } else {
        snippet
    }
}

// 构建 MCP 服务器上下文消息
// 作用：获取已连接的 MCP 服务器列表，注入到上下文让 AI 知道有哪些外部工具可用。
// 处理逻辑：
// 1. 调用 connected_server_catalog 获取已连接服务器列表
// 2. 如果没有服务器则返回 None
// 3. 格式化为带标记的上下文消息，包含服务器名称、类型、工具数量
// 用途：让 AI 知道可以调用哪些 MCP 工具，但不直接暴露工具细节。
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

// 判断是否是会话开始回合
// 作用：检查消息列表是否是新会话的第一轮对话。
// 判断标准：没有 assistant 消息，且 user 消息不超过 1 条。
// 用途：决定是否注入 session_start_hooks（会话开始时的初始化上下文）。
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
) -> Result<(), String> {
    let post_compact_hook =
        crate::llm::services::hooks::run_post_compact_hooks(app, conversation_id);
    if let Some(error) = post_compact_hook.override_error {
        return Err(error);
    }
    if !post_compact_hook.additional_messages.is_empty() {
        messages.extend(post_compact_hook.additional_messages);
    }
    Ok(())
}

// 从消息列表中移除每轮动态注入的上下文消息（RAG、MCP catalog、会话恢复、全局记忆、所有 hook 注入）。
// 保存快照前调用，确保快照只包含真实对话内容。
fn strip_injected_context(messages: &mut Vec<Message>) {
    const MARKERS: &[&str] = &[
        MCP_SERVER_CONTEXT_MARKER,
        "[Session Restore Context]",
        "[Global Memory]",
        "[Session Files]",
        "[Project Context]",
        "[Phase]",
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

// 入口函数：发送用户聊天消息，驱动一整个 agent turn。
// 它负责把“前端输入 → 后端可信模型上下文 → provider 流式输出 → 工具环回 → snapshot 持久化”
// 收敛成一条可恢复、可取消、ToolUse/ToolResult 成对合法的主流程。
//
// 核心职责：
// 1) 从 turn snapshot 恢复可信历史；非首轮缺 snapshot 时直接失败，不用前端 UI 历史兜底。
// 2) 每轮重新注入动态上下文（global memory / hooks / session RAG / MCP catalog），保存前再剥离。
// 3) 循环调用 provider，把 assistant 输出、tool_use、tool_result 和工具 side-channel 消息回灌进 current_messages。
// 4) 处理 cancelled / needs_user_input / stop hook 阻断 / provider error / prompt too long reactive compact。
// 5) 正常收尾时执行 session_end_hooks、保存 clean snapshot，并向前端发送最终 stop 事件。
//
// send_chat_message
//     │
//     ├─ 1. 回合前输入准备
//     │       ├─ latest user text                  → 提取 RAG query / 原始上传文件行
//     │       ├─ run_user_prompt_submit_hooks      → 追加提示提交上下文
//     │       └─ (首轮) run_session_start_hooks    → 追加会话开始上下文
//     │
//     ├─ 2. 可信历史恢复
//     │       ├─ load_turn_snapshot                → 恢复上一轮完整模型上下文
//     │       ├─ strip_injected_context            → 移除上一轮动态注入内容
//     │       ├─ append current turn input/hooks   → 只追加本轮新增用户输入
//     │       └─ missing snapshot on non-first turn → Err
//     │
//     ├─ 3. 请求前上下文构建
//     │       ├─ context_assembler                 → 注入 global memory；正常 agent 流不注入 session_restore
//     │       ├─ run_pre_compact_hooks             → 压缩前临时上下文
//     │       ├─ compact                           → proactive compact / 大型 tool_result 瘦身
//     │       ├─ run_post_compact_hooks            → 仅在发生 compact 后追加
//     │       ├─ session RAG                       → 当前会话文档检索 / 直接附件上下文
//     │       └─ MCP server catalog                → 注入已连接 MCP server 概览
//     │
//     ├─ 4. 主循环 loop
//     │       ├─ cancellation check                → cancelled
//     │       ├─ apply_tool_result_context_editing → 清理较早的大型工具结果
//     │       ├─ provider.send_request             → 流式输出 + 工具执行
//     │       │       ├─ prompt too long           → reactive compact 后重试一次
//     │       │       └─ other error               → 保存 partial snapshot + error hooks + stop(error)
//     │       ├─ provider returned cancelled       → 保留 partial，补齐缺失 ToolResult，写入 interrupted marker
//     │       ├─ merge provider_result.messages    → 回灌 assistant / tool_result / side-channel messages
//     │       ├─ tool_call invariant check         → tool_use stop_reason 必须带 ToolResult
//     │       ├─ needs_user_input                  → break
//     │       ├─ prevent_continuation              → stop_hook_prevented
//     │       ├─ has_tool_result                   → continue，让模型消费工具结果
//     │       └─ no tool_result
//     │               ├─ run_stop_hooks
//     │               ├─ added_context             → current_messages.extend + continue
//     │               └─ completed                 → break
//     │
//     └─ 5. 回合收尾（非 provider error 路径）
//             ├─ run_session_end_hooks             → 可覆盖 stop_reason
//             ├─ strip_injected_context
//             ├─ save_turn_snapshot                → 持久化 clean model context
//             └─ emit final stop                   → return Ok
pub async fn send_chat_message(
    app: AppHandle,
    conversation_id: Option<String>,
    messages: Vec<Message>,
    agent_mode: AgentMode,
) -> Result<(), String> {
    // 判断是否是会话第一轮，决定是否注入 session_start_hooks
    let session_start_turn = is_session_start_turn(&messages);
    // 记录前端传入消息数量，用于之后从 turn_messages 中定位"本轮新消息"起始位置。
    let frontend_msg_count = messages.len();
    let mut turn_messages = messages;

    // 执行用户提交钩子，可能追加额外上下文（如用户配置的提示前缀）
    let prompt_submit_hook =
        crate::llm::services::hooks::run_user_prompt_submit_hooks(&app, conversation_id.as_deref());
    // 钩子返回错误时直接中断，不继续执行
    if let Some(error) = prompt_submit_hook.override_error {
        return Err(error);
    }
    // 钩子产生的额外消息追加到对话列表（如用户配置的提示前缀）
    if !prompt_submit_hook.additional_messages.is_empty() {
        turn_messages.extend(prompt_submit_hook.additional_messages);
    }

    // 如果是会话第一轮，执行会话开始钩子，注入初始化上下文（如用户偏好、项目规则等）
    if session_start_turn {
        let session_start_hook =
            crate::llm::services::hooks::run_session_start_hooks(&app, conversation_id.as_deref());
        if let Some(error) = session_start_hook.override_error {
            return Err(error);
        }
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
    if let Some(error) = pre_compact_hook.override_error {
        return Err(error);
    }
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
    .await?;

    // 只有真的发生 compact 时才跑 post compact hook。
    // 当前 post hook 同样是配置文本注入：settings.hook_env["NOVA_POST_COMPACT_HOOK_CONTEXT"]。
    // compact 通知只用于前端展示本轮节省了多少上下文，不改变历史来源。
    let did_compact = compact_outcome.did_compact();
    let mut current_messages = compact_outcome.messages;
    if did_compact {
        apply_post_compact_hook(&app, conversation_id.as_deref(), &mut current_messages)?;
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

    // MCP catalog 也是本轮临时上下文：
    // 告诉模型当前连接了哪些 MCP server，但不提前展开具体工具。
    // 模型后续需要时再通过 mcp_auth/list_tools/call_tool 走正式工具流。
    if let Some(mcp_context) = build_mcp_server_context_message(&app).await {
        current_messages.push(mcp_context);
    }
    // println!("current_messages:{:?}", current_messages);

    let mut provider = LlmClient::new(&app)?;

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
        let model = crate::command::settings::get_settings(app.clone())?
            .active_provider_profile()
            .model;

        let window_tokens =
            crate::llm::utils::model_context::get_context_window_tokens(&model) as i64;

        if !crate::llm::utils::model_context::supports_image_input(&model) {
            current_messages = strip_images_to_text(&current_messages);
        }
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

        // 请求 provider 前先估算当前 prompt 占用，并通知前端更新 context window UI。
        // 这是本地估算值，不参与模型调用；provider 返回真实 usage 后会再用 actual 数据校正。

        let prompt_estimate = provider
            .estimate_prompt_tokens(
                &app,
                &current_messages,
                agent_mode,
                conversation_id.as_deref(),
            )
            .map_err(|error| error.message)?;
        let request_input_estimate = prompt_estimate.input_tokens;
        emit_context_usage_event(
            &app,
            conversation_id.as_deref(),
            request_input_estimate,
            window_tokens as u32,
            prompt_estimate.source,
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
                        )?;
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
                        let _ =
                            crate::llm::history::save_turn_snapshot(&app, conv_id, &snapshot).await;
                    }
                }

                let error_hook = crate::llm::services::hooks::run_error_hooks(
                    &app,
                    &e,
                    conversation_id.as_deref(),
                );
                let error_text = error_hook.override_error.unwrap_or_else(|| e.clone());
                // 出错直接通知前端并终止回合，走统一的 TurnOutcome::error 路径。
                // 错误详情通过 emit_backend_error 上报，stop 事件不再重复透传原始文本。
                emit_backend_error(
                    &app,
                    "llm.query_engine",
                    error_text.clone(),
                    Some("provider.send_request"),
                );
                break TurnOutcome::error(error_text);
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
                                is_error: true,
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
            provider_result.cache_read_tokens,
            provider_result.cache_creation_tokens,
            provider_result.cost.as_ref(),
            input_token_source,
        );
        let log_cost = provider_result
            .cost
            .as_ref()
            .map(|c| c.total_cost_usd.as_str());
        let _ = crate::llm::services::token_usage_log::log_token_usage(
            &app,
            conversation_id.as_deref(),
            provider.model(),
            Some(provider.provider_name()),
            input_tokens.unwrap_or(0),
            provider_result.output_tokens.unwrap_or(0),
            provider_result.cache_read_tokens.unwrap_or(0),
            provider_result.cache_creation_tokens.unwrap_or(0),
            log_cost,
            Some(input_token_source),
        )
        .await;
        emit_token_debug_event(
            &app,
            conversation_id.as_deref(),
            prompt_estimate.source,
            request_input_estimate,
            provider_result.input_tokens,
            provider_result.output_tokens,
            prompt_estimate.tool_count,
        );

        // 若 provider 返回了实际 input_tokens，用真实值刷新上下文用量显示。
        // 根据 Anthropic 文档：total_input = input_tokens + cache_read + cache_creation
        if let Some(actual_input) = provider_result.input_tokens {
            let total_input = actual_input
                + provider_result.cache_read_tokens.unwrap_or(0)
                + provider_result.cache_creation_tokens.unwrap_or(0);
            emit_context_usage_event(
                &app,
                conversation_id.as_deref(),
                total_input,
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
            // 保存 partial snapshot：provider 返回了 tool_use 但缺少对应的 ToolResult，
            // current_messages 已包含 provider 输出，保存以避免下轮上下文丢失。
            if let Some(conv_id) = conversation_id.as_deref() {
                let mut snapshot = current_messages.clone();
                strip_injected_context(&mut snapshot);
                let _ = crate::llm::history::save_turn_snapshot(&app, conv_id, &snapshot).await;
            }
            break TurnOutcome::error(msg);
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
            if let Some(error) = stop_hook_result.override_error {
                finalize_turn_on_error(&app, conversation_id.as_deref(), &error);
                return Err(error);
            }
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

    // Error 路径：跳过 session_end_hooks 和完整 snapshot 保存，
    // 因为回合未正常完成，partial snapshot 已在循环内保存。
    if matches!(final_outcome.turn_state, state_machine::TurnState::Error) {
        crate::llm::services::live_turns::mark_terminal(
            conversation_id.as_deref(),
            final_outcome.turn_state.as_event_state(),
        );
        app.emit(
            "chat-stream",
            ChatMessageEvent {
                r#type: "stop".into(),
                text: None,
                tool_use_id: None,
                tool_use_name: None,
                tool_use_input: None,
                tool_result: None,
                tool_is_error: None,
                token_usage: None,
                stop_reason: Some(final_outcome.stop_reason.clone()),
                turn_state: Some(final_outcome.turn_state.as_event_state().to_string()),
                conversation_id: conversation_id.clone(),
            },
        )
        .ok();
        return Err(final_outcome.stop_reason);
    }

    // 非 Error 路径：执行 session_end_hooks、保存完整 snapshot、发送 stop 事件。
    let session_end_hook = crate::llm::services::hooks::run_session_end_hooks(
        &app,
        &final_outcome.stop_reason,
        conversation_id.as_deref(),
    );
    if let Some(error) = session_end_hook.override_error {
        finalize_turn_on_error(&app, conversation_id.as_deref(), &error);
        return Err(error);
    }
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
    crate::llm::services::live_turns::mark_terminal(
        conversation_id.as_deref(),
        final_outcome.turn_state.as_event_state(),
    );
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
            tool_is_error: None,
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

// 错误路径收尾：标记会话为 error 终态并发送 stop 事件。
// 用于早返回路径（stop hook / session_end hook 报错），避免跳过 mark_terminal
// 导致 live_turns 状态卡在 "running" 且前端收不到 stop 事件。
fn finalize_turn_on_error(
    app: &AppHandle,
    conversation_id: Option<&str>,
    stop_reason: &str,
) {
    crate::llm::services::live_turns::mark_terminal(conversation_id, "error");
    app.emit(
        "chat-stream",
        ChatMessageEvent {
            r#type: "stop".into(),
            text: None,
            tool_use_id: None,
            tool_use_name: None,
            tool_use_input: None,
            tool_result: None,
            tool_is_error: None,
            token_usage: None,
            stop_reason: Some(stop_reason.to_string()),
            turn_state: Some("error".to_string()),
            conversation_id: conversation_id.map(str::to_string),
        },
    )
    .ok();
}
