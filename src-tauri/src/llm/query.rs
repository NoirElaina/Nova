use tauri::{AppHandle, Emitter};
use tracing::info;

use std::collections::HashSet;

use crate::llm::providers::LlmClient;
use crate::llm::query_engine::ChatMessageEvent;
use crate::llm::services::compact;
use crate::llm::utils::token_counter;
use crate::llm::types::{AgentMode, Content, ContentBlock, Message, Role};
use crate::llm::utils::context_injection;
use crate::llm::utils::error_event::emit_backend_error;
use crate::llm::utils::pricing::TurnCostBreakdown;

mod state_machine;

use state_machine::TurnOutcome;

/// 会话事件日志写入：best-effort，失败仅告警不阻断对话主流程。
async fn log_session_event(
    app: &AppHandle,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
    event: crate::llm::session_log::SessionEvent,
) {
    let Some(conv_id) = conversation_id else {
        return;
    };
    if let Err(error) =
        crate::llm::session_log::append_event(app, conv_id, turn_id, &event).await
    {
        tracing::warn!(error = %error, conversation_id = %conv_id, event = event.type_name(), "session event append failed");
    }
}

/// 把一批新增模型消息写入事件日志（按角色归类；注入类上下文不应出现在这里）。
/// usage 非空时归到批内最后一条助手消息（与该请求的计费口径一致）；
/// user_attachments 非空时挂到批内最后一条用户消息（UI 展示用）。
async fn log_new_model_messages(
    app: &AppHandle,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
    messages: &[Message],
    usage: Option<(i64, serde_json::Value)>,
    user_attachments: Option<Vec<crate::llm::commands::types::HistoryAttachment>>,
) {
    if messages.is_empty() {
        return;
    }
    let mut events: Vec<crate::llm::session_log::SessionEvent> = messages
        .iter()
        .cloned()
        .map(crate::llm::session_log::SessionEvent::from_model_message)
        .collect();
    if let Some((tokens, cost)) = usage {
        if let Some(last_assistant) = events
            .iter_mut()
            .rev()
            .find(|e| matches!(e, crate::llm::session_log::SessionEvent::AssistantMessage { .. }))
        {
            if let crate::llm::session_log::SessionEvent::AssistantMessage { token_usage, cost: cost_slot, .. } = last_assistant {
                *token_usage = Some(tokens);
                *cost_slot = Some(cost);
            }
        }
    }
    if let Some(attachments) = user_attachments {
        if !attachments.is_empty() {
            if let Some(last_user) = events
                .iter_mut()
                .rev()
                .find(|e| matches!(e, crate::llm::session_log::SessionEvent::UserMessage { .. }))
            {
                if let crate::llm::session_log::SessionEvent::UserMessage { attachments: slot, .. } = last_user {
                    *slot = Some(attachments);
                }
            }
        }
    }
    let Some(conv_id) = conversation_id else {
        return;
    };
    if let Err(error) = crate::llm::session_log::append_events(app, conv_id, turn_id, &events).await {
        tracing::warn!(error = %error, conversation_id = %conv_id, count = events.len(), "session events append failed");
    }
}

/// 追加到中断回复末尾的标记文本（对应取消路径的“（已取消当前轮）”，前端同文案）。
pub const ERROR_INTERRUPTED_MARKER: &str = "（本轮因错误中断）";

/// 网络/传输层瞬时错误的自动重发配置：5 秒间隔、最多 3 次，同一回合内完成。
const TRANSPORT_RETRY_INTERVAL_SECS: u64 = 5;
const MAX_TRANSPORT_RETRIES: u32 = 3;

/// 判定是否为可自动重发的网络/传输层瞬时错误。
/// 业务错误（4xx、协议解析失败、流内 error 事件）不重试，避免无意义消耗。
fn is_transient_transport_error(msg: &str) -> bool {
    msg.contains("stream chunk error")
        || msg.contains("error sending request")
        || msg.contains("empty assistant message")
        || msg.contains("incomplete SSE event")
        || msg.contains("API Error [429")
        || msg.contains("API Error [500")
        || msg.contains("API Error [502")
        || msg.contains("API Error [503")
        || msg.contains("API Error [504")
}

/// 把中断标记追加到部分输出的最后一条助手消息末尾：
/// 尾块是 Text 时直接续写，否则新增一个 Text 块。
/// 让 UI 与模型都能看出这是一条半截回复，下一轮不会把它当完整回答继续推理。
fn append_error_interruption_marker(messages: &mut [Message]) {
    let Some(assistant) = messages
        .iter_mut()
        .rev()
        .find(|m| matches!(m.role, Role::Assistant))
    else {
        return;
    };
    if let Content::Blocks(blocks) = &mut assistant.content {
        match blocks.last_mut() {
            Some(ContentBlock::Text { text }) => {
                text.push_str("\n\n");
                text.push_str(ERROR_INTERRUPTED_MARKER);
            }
            _ => blocks.push(ContentBlock::Text {
                text: ERROR_INTERRUPTED_MARKER.to_string(),
            }),
        }
    }
}

/// 写压缩检查点：base_context = 压缩后的起点上下文。
/// 注入块（会话文件 / MCP / 阶段）是合法历史的一部分，随基线原样保留。
/// 压缩后历史整体变短，同步重置缓存击穿检测基线，防止自然下降误报。
async fn log_compact_boundary(
    app: &AppHandle,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
    context: &[Message],
    level: &str,
    tokens_before: u32,
    tokens_after: u32,
) {
    log_session_event(
        app,
        conversation_id,
        turn_id,
        crate::llm::session_log::SessionEvent::CompactBoundary {
            base_context: context.to_vec(),
            summary: format!("[{}] context compaction", level),
            level: level.to_string(),
            tokens_before,
            tokens_after,
        },
    )
    .await;
    crate::llm::services::prompt_cache_break::reset_baseline(conversation_id);
}

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
    let total_input = input_tokens
        .unwrap_or(0)
        .saturating_add(cache_read_tokens.unwrap_or(0))
        .saturating_add(cache_creation_tokens.unwrap_or(0));
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
// 用途：遗留清理列表扫描文本内容来移除旧时代停产标记（[Project Context] / [Global Memory]）。
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

// 判断是否是会话开始回合
// 作用：检查消息列表是否从未成功完成过一轮对话。
// 判断标准：没有任何 assistant 消息（不限制 user 数量）。
// 用途：决定是否注入 session_start_hooks，以及无事件日志时是否允许用前端输入启动。
// 不限制 user 数量的原因：首轮发送失败后重发，历史里会有 2 条 user 消息，
// 若按数量判定会被误判为非首轮，撞上"无事件日志"错误且 hooks 不再注入。
fn is_session_start_turn(messages: &[Message]) -> bool {
    messages.iter().all(|m| m.role != Role::Assistant)
}

async fn apply_post_compact_hook(
    app: &AppHandle,
    conversation_id: Option<&str>,
    messages: &mut Vec<Message>,
) -> Result<(), String> {
    let post_compact_hook =
        crate::llm::services::hooks::run_post_compact_hooks(app, conversation_id).await;
    if let Some(error) = post_compact_hook.override_error {
        return Err(error);
    }
    if !post_compact_hook.additional_messages.is_empty() {
        messages.extend(post_compact_hook.additional_messages);
    }
    Ok(())
}

// 遗留清理列表：仅针对旧剥离+重注入时代烙进 compact 边界 base_context 的已停产标记。
// 新代码永不生产这两个标记（[Project Context] 已删除，[Global Memory] 改由系统提示词承载），
// 剥离是确定性纯函数（同一事件流每轮结果逐字节一致），不产生新的缓存分叉。
// 注意：新注入块标记（会话文件 / MCP / 阶段 / hooks）绝不在此列——它们是合法历史。
fn strip_legacy_injected_context(messages: &mut Vec<Message>) {
    const LEGACY_MARKERS: &[&str] = &["[Project Context]", "[Global Memory]"];
    messages.retain(|m| {
        let text = text_from_content(&m.content);
        !LEGACY_MARKERS
            .iter()
            .any(|marker| text.starts_with(marker))
    });
}

// 入口函数：发送用户聊天消息，驱动一整个 agent turn。
// 它负责把“前端输入 → 事件日志重建的可信上下文 → provider 流式输出 → 工具环回 → 事件日志增量落盘”
// 收敛成一条可恢复、可取消、ToolUse/ToolResult 成对合法的主流程。
//
// 核心职责：
// 1) 重放会话事件日志恢复可信历史；非首轮无事件时直接失败，不用前端 UI 历史兜底。
// 2) 缓存友好的上下文注入：会话文件 / MCP 目录 / 阶段提示由 context_injection 差量持久化，
//    写入即永久、状态未变零注入；hooks 注入内容随回合原样持久化（用户意图的组成部分）。
// 3) 循环调用 provider，把 assistant 输出、tool_use、tool_result 和工具 side-channel 消息回灌进 current_messages 并写入事件日志。
// 4) 处理 cancelled / needs_user_input / stop hook 阻断 / provider error / prompt too long reactive compact。
// 5) 正常收尾时执行 session_end_hooks，并向前端发送最终 stop 事件。
//
// send_chat_message
//     │
//     ├─ 1. 回合前输入准备 + 事件写入（TurnStart/注入块/用户输入/标题与活跃度）
//     │       ├─ latest user text                  → 提取 RAG query / 原始上传文件行
//     │       ├─ run_user_prompt_submit_hooks      → 追加提示提交上下文（随回合持久化）
//     │       └─ (首轮) run_session_start_hooks    → 追加会话开始上下文（随回合持久化）
//     │
//     ├─ 2. 可信历史恢复（事件日志重放，遇压缩检查点重置）
//     │       ├─ load_events + reconstruct         → 重建上一轮完整模型上下文（含持久化的注入块）
//     │       ├─ strip_legacy_injected_context     → 仅清理旧时代停产标记（确定性）
//     │       ├─ context_injection 差量注入        → 变化才追加，先落 ContextMessage 事件
//     │       ├─ append current turn input/hooks   → 只追加本轮新增用户输入（注入块位于其前）
//     │       └─ 无事件且非首轮                     → Err（旧数据不兼容）
//     │
//     ├─ 3. 请求前上下文构建
//     │       ├─ run_pre_compact_hooks             → 压缩前临时上下文
//     │       ├─ run_pre_compact_hooks             → 压缩前临时上下文
//     │       ├─ compact                           → proactive compact / 大型 tool_result 瘦身（写 CompactBoundary 事件）
//     │       ├─ run_post_compact_hooks            → 仅在发生 compact 后追加
//     │       └─ session RAG                       → 当前会话文档检索 / 直接附件上下文
//     │       └─ MCP server catalog                → 注入已连接 MCP server 概览
//     │
//     ├─ 4. 主循环 loop（provider 输出增量写入事件日志）
//     │       ├─ cancellation check                → cancelled
//     │       ├─ apply_tool_result_context_editing → 清理较早的大型工具结果（仅请求副本，不改日志）
//     │       ├─ provider.send_request             → 流式输出 + 工具执行（ToolCall/ToolResult 事件）
//     │       │       ├─ prompt too long           → reactive compact（写 CompactBoundary）后重试一次
//     │       │       └─ other error               → 部分输出入日志 + error hooks + stop(error)
//     │       ├─ provider returned cancelled       → 保留 partial 入日志，补齐缺失 ToolResult，写入 interrupted marker
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
//             ├─ TurnEnd 事件
//             └─ emit final stop                   → return Ok
pub async fn send_chat_message(
    app: AppHandle,
    conversation_id: Option<String>,
    messages: Vec<Message>,
    agent_mode: AgentMode,
    attachments: Option<Vec<crate::llm::commands::types::HistoryAttachment>>,
) -> Result<(), String> {
    // 轮次开始：从 DB 刷新该会话挂载的智能体缓存（写穿透兜底，防冷启动读不到）。
    // 之后 provider adapter / system_prompt / SkillTool 的同步读都命中缓存。
    if let Some(conv_id) = conversation_id.as_deref() {
        crate::llm::services::agent_bundles::refresh_single_conversation_agent(&app, conv_id)
            .await;
    }

    // 判断是否是会话第一轮，决定是否注入 session_start_hooks
    let session_start_turn = is_session_start_turn(&messages);
    // 记录前端传入消息数量，用于之后从 turn_messages 中定位"本轮新消息"起始位置。
    let frontend_msg_count = messages.len();
    let mut turn_messages = messages;

    // 执行用户提交钩子，可能追加额外上下文（如用户配置的提示前缀）
    let prompt_submit_hook =
        crate::llm::services::hooks::run_user_prompt_submit_hooks(&app, conversation_id.as_deref()).await;
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
            crate::llm::services::hooks::run_session_start_hooks(&app, conversation_id.as_deref()).await;
        if let Some(error) = session_start_hook.override_error {
            return Err(error);
        }
        if !session_start_hook.additional_messages.is_empty() {
            turn_messages.extend(session_start_hook.additional_messages);
        }
    }

    // ── 会话事件日志：回合开始 + 差量注入块 + 本轮新输入 ──
    // 事件顺序固定为 TurnStart → 注入块（如有）→ 用户消息，
    // 使注入块落在"历史末尾与新用户消息之间"：下一轮请求的字节前缀恰好是本轮末次请求的完整前缀。
    let turn_id = uuid::Uuid::new_v4().to_string();
    log_session_event(
        &app,
        conversation_id.as_deref(),
        Some(&turn_id),
        crate::llm::session_log::SessionEvent::TurnStart {
            turn_id: turn_id.clone(),
        },
    )
    .await;
    // ── 可信历史重建 + 差量注入 + 事件日志写入 ──
    // 缓存友好约束（对标 codex world_state）：
    // - 重建结果只清理旧时代停产标记（确定性剥离，不产生分叉），注入块作为合法历史原样保留；
    // - 事件顺序固定为 TurnStart → 注入块（如有）→ 用户消息，注入块落在"历史末尾与新用户消息之间"：
    //   下一轮请求的字节前缀恰好是本轮末次请求的完整前缀，跨回合全量命中；
    // - 差量注入与历史末块比对，状态未变时零注入；
    // - hooks 注入内容随回合持久化，请求与历史逐字节一致。
    let Some(conv_id) = conversation_id.as_deref() else {
        return Err("send_chat_message requires conversation_id".to_string());
    };
    let events = crate::llm::session_log::load_events(&app, conv_id).await?;
    if events.is_empty() && !session_start_turn {
        return Err(format!(
            "会话 {} 无事件日志且不是首轮请求，拒绝使用前端历史兜底（旧数据不兼容，请新开对话）",
            conv_id
        ));
    }
    // 重建历史（含持久化的注入块）并计算差量注入；先落注入事件、再落用户消息事件。
    let mut reconstructed = crate::llm::session_log::projection::reconstruct_model_context(&events);
    strip_legacy_injected_context(&mut reconstructed);
    let injections =
        context_injection::ensure_injections(&app, Some(conv_id), &reconstructed).await;
    for message in &injections {
        log_session_event(
            &app,
            conversation_id.as_deref(),
            Some(&turn_id),
            crate::llm::session_log::SessionEvent::ContextMessage {
                message: message.clone(),
            },
        )
        .await;
    }
    let new_input_start = frontend_msg_count.saturating_sub(1);
    // 错误后重发去重：上一回合零输出报错时（连接失败/空响应），用户消息已落库，
    // 重建上下文末尾就是它；用户原样重发同一句话时不再重复落库与追加，
    // 否则历史出现重复气泡，模型上下文还会出现连续两条同角色消息。
    // 限制：本轮不带新附件（原消息已含同样的话，附件版本差异不走此路径）。
    let duplicate_retry = attachments.as_ref().map(|a| a.is_empty()).unwrap_or(true)
        && turn_messages
            .get(new_input_start)
            .zip(reconstructed.last())
            .map(|(incoming, tail)| {
                matches!(incoming.role, Role::User)
                    && matches!(tail.role, Role::User)
                    && crate::llm::session_log::projection::content_text(&incoming.content)
                        == crate::llm::session_log::projection::content_text(&tail.content)
            })
            .unwrap_or(false);
    {
        let new_inputs: Vec<Message> =
            turn_messages[new_input_start + usize::from(duplicate_retry)..].to_vec();
        log_new_model_messages(
            &app,
            conversation_id.as_deref(),
            Some(&turn_id),
            &new_inputs,
            None,
            attachments,
        )
        .await;
        // 后端接管持久化维护：刷新会话活跃时间与标题（首条用户消息时派生）。
        let latest_user_text = new_inputs
            .iter()
            .rev()
            .find(|m| matches!(m.role, Role::User))
            .map(|m| crate::llm::session_log::projection::content_text(&m.content));
        if let Some(conv_id) = conversation_id.as_deref() {
            if let Err(error) = crate::llm::history::refresh_conversation_activity(
                &app,
                conv_id,
                latest_user_text.as_deref(),
            )
            .await
            {
                tracing::warn!(error = %error, conversation_id = %conv_id, "refresh conversation activity failed");
            }
        }
    }

    // 组装本轮工作上下文：重建历史 + 注入块 + 本轮新增输入（含 hooks 消息）。
    let working_messages = {
        let mut ctx = reconstructed;
        ctx.extend(injections);
        // 前端消息只用来定位本轮新增输入；历史必须来自事件日志。
        // hooks 已追加到 turn_messages 尾部，因此从最新用户消息开始整体追加；
        // 与事件日志一致：命中错误重发去重时跳过重复的用户消息本身。
        let new_start = new_input_start + usize::from(duplicate_retry);
        ctx.extend_from_slice(&turn_messages[new_start..]);
        ctx
    };

    let mut current_messages = working_messages;

    // 压缩前挂钩：由 hooks.toml 声明（上下文注入/命令挂钩），
    // 注入的消息放在 compact 前，让它也参与 token 估算和压缩决策。
    let pre_compact_hook =
        crate::llm::services::hooks::run_pre_compact_hooks(&app, conversation_id.as_deref()).await;
    if let Some(error) = pre_compact_hook.override_error {
        return Err(error);
    }
    if !pre_compact_hook.additional_messages.is_empty() {
        current_messages.extend(pre_compact_hook.additional_messages);
    }

    // 根据当前模型上下文窗口选择压缩策略：
    // - none：不压缩；
    // - micro：本地截断/瘦身较大的 tool_result；
    // - full：先 micro，再用模型总结旧上下文并保留最近消息。
    // 返回的 messages 会成为本轮真正继续往下传的 current_messages。
    let compact_outcome = compact::compact_messages_for_turn_with_report(
        &app,
        conversation_id.as_deref(),
        &current_messages,
    )
    .await?;

    // 只有真的发生 compact 时才跑 post compact 挂钩（hooks.toml 声明）。
    // compact 通知只用于前端展示本轮节省了多少上下文，不改变历史来源。
    let did_compact = compact_outcome.did_compact();
    current_messages = compact_outcome.messages;
    if did_compact {
        apply_post_compact_hook(&app, conversation_id.as_deref(), &mut current_messages).await?;
        let after_tokens = clamp_i64_to_u32(token_counter::count_messages(&current_messages));
        emit_context_compact_event(
            &app,
            conversation_id.as_deref(),
            compact_outcome.level,
            "自动压缩历史上下文，减少发送给模型的背景信息体积。",
            clamp_i64_to_u32(compact_outcome.estimated_tokens),
            after_tokens,
        );
        // 事件日志压缩检查点：重建模型上下文时丢弃旧事件，以 base_context 为起点。
        log_compact_boundary(
            &app,
            conversation_id.as_deref(),
            Some(&turn_id),
            &current_messages,
            compact_outcome.level,
            clamp_i64_to_u32(compact_outcome.estimated_tokens),
            after_tokens,
        )
        .await;
    }

    let mut provider = LlmClient::new(&app)?;

    // 3. 主循环：调用 provider.send_request（流式），并根据 tool 执行情况决定是否继续下一步。
    //    - 如果发生工具调用，结果会被“注入”到 current_messages 继续下一轮。
    //    - 如果 provider 返回 needs_user_input / 无工具结果，则结束。
    let mut has_attempted_reactive_compact = false;
    // 网络层瞬时错误的自动重发计数（同一回合内，5 秒一次，上限 MAX_TRANSPORT_RETRIES）。
    let mut transport_retry_count: u32 = 0;
    let mut final_outcome = loop {
        // 若收到取消请求，则立即以 cancelled 结束。
        if crate::llm::cancellation::is_cancelled(conversation_id.as_deref()) {
            break TurnOutcome::cancelled();
        }

        // 每次请求 provider 前重新读取当前模型配置，拿到该模型的上下文窗口大小。
        // 模型可能在设置中切换，因此这里不复用回合开始时的窗口值。
        // 窗口：用户 per-model 覆盖 > 内置 JSON > 默认。
        let settings = crate::command::settings::load_settings(&app)?;
        let model = settings.active_provider_profile().model;
        let window_tokens = settings.context_window_for_model(&model) as i64;

        // 会话途中压缩：与轮开始同一强度（≥80% Micro / ≥90% Full）。
        // 单轮内多轮工具调用的结果增长只有轮开始压缩覆盖不到，这里每次请求前兜住。
        // 压缩后的上下文随 CompactBoundary 事件持久化；
        // 最近一轮尚未消费的工具结果保留原文。
        let mid_compact = compact::compact_messages_mid_turn(
            &app,
            conversation_id.as_deref(),
            &mut current_messages,
            window_tokens,
        )
        .await;
        if mid_compact.applied {
            apply_post_compact_hook(&app, conversation_id.as_deref(), &mut current_messages).await?;
            emit_context_compact_event(
                &app,
                conversation_id.as_deref(),
                mid_compact.level,
                "会话途中自动压缩上下文，避免工具结果累积占满窗口。",
                clamp_i64_to_u32(mid_compact.tokens_before),
                clamp_i64_to_u32(mid_compact.tokens_after),
            );
            log_compact_boundary(
                &app,
                conversation_id.as_deref(),
                Some(&turn_id),
                &current_messages,
                mid_compact.level,
                clamp_i64_to_u32(mid_compact.tokens_before),
                clamp_i64_to_u32(mid_compact.tokens_after),
            )
            .await;
        }

        // 不支持图片输入的模型：剥离图片为占位文本，但只在临时变量上操作，
        // 不覆盖 current_messages。否则写入事件日志时会丢失原始图片数据，
        // 即使用户切回支持图片的模型也无法恢复。
        let messages_for_provider: Vec<Message> =
            if crate::llm::utils::model_context::supports_image_input(&model) {
                current_messages.clone()
            } else {
                strip_images_to_text(&current_messages)
            };
        // 工具结果上下文编辑：专门处理较早、较大的 tool_use/tool_result 对。
        // 它不同于前面的整体 compact；这里在主循环每次 provider 请求前执行，
        // 用于防止多轮工具调用后旧工具输出持续占满上下文窗口。
        let context_editing =
            compact::apply_tool_result_context_editing(&messages_for_provider, window_tokens);
        let messages_for_provider = if context_editing.applied {
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
            context_editing.messages
        } else {
            messages_for_provider
        };

        // 缓存击穿检测（请求侧）：记录系统提示词/工具集/模型/提供商指纹，
        // 与上一次请求比对供响应侧归因；三家提供商共用此处一处挂接。
        {
            let system_for_fingerprint =
                crate::llm::utils::system_prompt::load_system_prompt(
                    &app,
                    agent_mode,
                    conversation_id.as_deref(),
                )
                .unwrap_or_default();
            let tools_for_fingerprint =
                crate::llm::tools::get_available_tools_for_agent(&app, conversation_id.as_deref());
            crate::llm::services::prompt_cache_break::record_request(
                conversation_id.as_deref(),
                provider.provider_name(),
                &system_for_fingerprint,
                &tools_for_fingerprint,
                &model,
            );
        }

        // 发起 provider 请求并等待结果。
        let (provider_result, prompt_estimate) = match provider
            .send_request(
                &app,
                &messages_for_provider,
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
                        &messages_for_provider,
                    )
                    .await
                    {
                        let before_tokens =
                            clamp_i64_to_u32(token_counter::count_messages(&messages_for_provider));
                        let after_tokens =
                            clamp_i64_to_u32(token_counter::count_messages(&recovered_messages));
                        // reactive_compact 是真正的上下文压缩恢复，压缩后的 messages
                        // 应该持久化（原始过长消息已无意义），所以覆盖 current_messages。
                        current_messages = recovered_messages;
                        apply_post_compact_hook(
                            &app,
                            conversation_id.as_deref(),
                            &mut current_messages,
                        )
                        .await?;
                        emit_context_compact_event(
                            &app,
                            conversation_id.as_deref(),
                            "reactive",
                            "模型提示上下文过长，已自动压缩后重试。",
                            before_tokens,
                            after_tokens,
                        );
                        log_compact_boundary(
                            &app,
                            conversation_id.as_deref(),
                            Some(&turn_id),
                            &current_messages,
                            "reactive",
                            before_tokens,
                            after_tokens,
                        )
                        .await;
                        has_attempted_reactive_compact = true;
                        continue;
                    }
                }

                // 网络层瞬时错误且零输出（尚无任何内容落库）时，同一回合内自动重发：
                // 5 秒一次、最多 3 次，每次通过 backend-warning 通知前端展示进度；
                // 已有部分输出时不重试——半截回复已落库，重试会造成内容重复。
                if provider_err.partial_messages.is_empty()
                    && is_transient_transport_error(&e)
                    && transport_retry_count < MAX_TRANSPORT_RETRIES
                {
                    transport_retry_count += 1;
                    crate::llm::utils::error_event::emit_backend_warning(
                        &app,
                        "llm.query_engine",
                        format!(
                            "网络异常（{}），{} 秒后自动重发（第 {}/{} 次）",
                            truncate_chars(&e, 80),
                            TRANSPORT_RETRY_INTERVAL_SECS,
                            transport_retry_count,
                            MAX_TRANSPORT_RETRIES
                        ),
                        Some("provider.auto_retry"),
                    );
                    // 等待期间响应取消：用户点停止则直接收敛为 cancelled。
                    let retry_cancel_token =
                        crate::llm::cancellation::get_token(conversation_id.as_deref());
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(
                            TRANSPORT_RETRY_INTERVAL_SECS,
                        )) => {}
                        _ = retry_cancel_token.cancelled() => {
                            break TurnOutcome::cancelled();
                        }
                    }
                    continue;
                }

                // 出错时无论如何都保存已输出内容（与主动取消同要求）：
                // 部分输出写入事件日志，并在末尾追加中断标记；一点输出都没有则无消息可存。
                if !provider_err.partial_messages.is_empty() {
                    let mut partial = provider_err.partial_messages.clone();
                    append_error_interruption_marker(&mut partial);
                    if let Some(conv_id) = conversation_id.as_deref() {
                        log_new_model_messages(
                            &app,
                            Some(conv_id),
                            Some(&turn_id),
                            &partial,
                            None,
                            None,
                        )
                        .await;
                    }
                } else {
                    // 零输出：中断标记进模型上下文（与取消路径一致的 ContextMessage 设计，
                    // 不在 UI 聊天气泡展示），告知下一轮上一条请求失败了。
                    log_session_event(
                        &app,
                        conversation_id.as_deref(),
                        Some(&turn_id),
                        crate::llm::session_log::SessionEvent::ContextMessage {
                            message: Message {
                                role: Role::User,
                                content: Content::Text(format!(
                                    "[Request interrupted by provider error: {}]",
                                    truncate_chars(&e, 200)
                                )),
                            },
                        },
                    )
                    .await;
                }

                let error_hook = crate::llm::services::hooks::run_error_hooks(
                    &app,
                    &e,
                    conversation_id.as_deref(),
                )
                .await;
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

        let request_input_estimate = prompt_estimate.input_tokens;
        emit_context_usage_event(
            &app,
            conversation_id.as_deref(),
            request_input_estimate,
            window_tokens as u32,
            prompt_estimate.source,
        );

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

            let interrupt_message = Message {
                role: Role::User,
                content: Content::Blocks(user_blocks),
            };
            current_messages.push(interrupt_message.clone());

            // 事件日志：记录半截输出。
            log_new_model_messages(
                &app,
                conversation_id.as_deref(),
                Some(&turn_id),
                &provider_result.messages,
                None,
                None,
            )
            .await;
            // 中断标记进模型上下文（下一轮模型需知道上一条是被打断的），
            // 但以 ContextMessage 落日志——不在 UI 聊天气泡展示。
            log_session_event(
                &app,
                conversation_id.as_deref(),
                Some(&turn_id),
                crate::llm::session_log::SessionEvent::ContextMessage {
                    message: interrupt_message,
                },
            )
            .await;

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
        // 缓存击穿检测（响应侧）：仅在提供商报告 cache_read 时判定。
        crate::llm::services::prompt_cache_break::check_response(
            &app,
            conversation_id.as_deref(),
            provider_result.cache_read_tokens,
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
                .saturating_add(provider_result.cache_read_tokens.unwrap_or(0))
                .saturating_add(provider_result.cache_creation_tokens.unwrap_or(0));
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
        // 事件日志：助手输出/工具结果回填等新增消息全部入日志；
        // 本请求的 token/成本归到批内最后一条助手消息。
        let usage_for_log = {
            let total = input_tokens
                .unwrap_or(0)
                .saturating_add(provider_result.output_tokens.unwrap_or(0));
            provider_result
                .cost
                .as_ref()
                .and_then(|c| serde_json::to_value(c).ok())
                .map(|cost| (total as i64, cost))
        };
        log_new_model_messages(&app, conversation_id.as_deref(), Some(&turn_id), &new_messages, usage_for_log, None).await;

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
            // provider 输出已在上方写入事件日志，下轮重建不会丢上下文。
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
            )
            .await;
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

    // 事件日志：回合终态（completed/cancelled/error/needs_user_input 均记录）。
    log_session_event(
        &app,
        conversation_id.as_deref(),
        Some(&turn_id),
        crate::llm::session_log::SessionEvent::TurnEnd {
            turn_id: turn_id.clone(),
            stop_reason: Some(final_outcome.stop_reason.clone()),
        },
    )
    .await;

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
    )
    .await;
    if let Some(error) = session_end_hook.override_error {
        finalize_turn_on_error(&app, conversation_id.as_deref(), &error);
        return Err(error);
    }
    if let Some(hooked_reason) = session_end_hook.stop_reason {
        final_outcome.stop_reason = hooked_reason;
    }

    // 模型上下文持久化已全部由事件日志承担（回合内增量写入），
    // 此处不再有快照保存步骤。

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
