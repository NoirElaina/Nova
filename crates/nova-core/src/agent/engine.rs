use crate::core::Core;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::context::assembler::ContextAssembler;
use crate::agent::events::AgentDomainEvent;
use crate::agent::state::CognitiveState;
use crate::provider::LlmClient;
use crate::agent::session::SessionEvent;
use crate::provider::types::{AgentMode, Content, ContentBlock, Message, Role};

/// 轮次执行交付结果
#[derive(Debug, Clone, Default)]
pub struct TurnOutcome {
    pub turn_id: String,
    pub stop_reason: String,
    pub total_tokens: u32,
    pub assistant_text: String,
}

pub use crate::agent::tools::orchestrator::{CircuitAction, ToolExecutionContext, ToolLoopBreaker, ToolOrchestrator};

/// 从 Assistant 消息中提取纯文本交付物
fn extract_assistant_text(messages: &[Message]) -> String {
    let mut text = String::new();
    for msg in messages {
        if msg.role != Role::Assistant {
            continue;
        }
        match &msg.content {
            Content::Text(t) => {
                if !t.trim().is_empty() {
                    text.push_str(t);
                    text.push('\n');
                }
            }
            Content::Blocks(blocks) => {
                for block in blocks {
                    if let ContentBlock::Text { text: t, .. } = block {
                        if !t.trim().is_empty() {
                            text.push_str(t);
                            text.push('\n');
                        }
                    }
                }
            }
        }
    }
    text.trim().to_string()
}

/// 现代化 Agent 核心引擎（Modern Cognitive Engine）
/// 严格践行单真实信源（Single Source of Truth），实现完整的 ReAct 状态流转与即时自愈验证。
pub struct AgentEngine {
    core: Core,
}

impl AgentEngine {
    pub fn new(core: Core) -> Self {
        Self { core }
    }

    fn build_user_message(
        &self,
        conversation_id: &str,
        user_prompt: &str,
        attachments: Option<&Vec<crate::agent::session::types::HistoryAttachment>>,
    ) -> Message {
        let mut text_content = user_prompt.to_string();
        let mut content_blocks = Vec::new();

        if let Some(atts) = attachments {
            for att in atts {
                if att.kind.as_deref() == Some("image") {
                    if let Some(ref data) = att.data {
                        let media_type = att
                            .media_type
                            .clone()
                            .or_else(|| att.mime_type.clone())
                            .unwrap_or_else(|| "image/png".to_string());
                        content_blocks.push(ContentBlock::Image {
                            source: crate::provider::types::ImageSource {
                                source_type: "base64".to_string(),
                                media_type,
                                data: data.clone(),
                            },
                        });
                    }
                } else if att.kind.as_deref() == Some("document") {
                    let mut doc_text = None;
                    if let Some(ref content) = att.content {
                        if !content.trim().is_empty() {
                            doc_text = Some(content.clone());
                        }
                    }
                    if doc_text.is_none() {
                        if let Some(ref file_path) = att.session_file_path {
                            if let Ok(extracted) =
                                crate::agent::capabilities::session_files::read_session_file(
                                    &self.core,
                                    conversation_id,
                                    file_path,
                                )
                            {
                                doc_text = Some(extracted);
                            }
                        }
                    }
                    if let Some(txt) = doc_text {
                        text_content.push_str(&format!(
                            "\n\n[Attached Document: {}]\n{}\n[End of Document]",
                            att.source_name, txt
                        ));
                    }
                }
            }
        }

        if content_blocks.is_empty() {
            Message {
                role: Role::User,
                content: Content::Text(text_content),
            }
        } else {
            let mut all_blocks = vec![ContentBlock::Text { text: text_content }];
            all_blocks.extend(content_blocks);
            Message {
                role: Role::User,
                content: Content::Blocks(all_blocks),
            }
        }
    }

    async fn inject_pending_steers(
        &self,
        conversation_id: &str,
        turn_id: &str,
        trace_id: &str,
        current_messages: &mut Vec<Message>,
    ) -> bool {
        let steers = crate::agent::session_runtime::drain_steers(Some(conversation_id));
        if steers.is_empty() {
            return false;
        }
        for steer in steers {
            let steer_msg = self.build_user_message(
                conversation_id,
                &steer.prompt,
                steer.attachments.as_ref(),
            );
            let _ = crate::agent::session::append_event(
                &self.core,
                conversation_id,
                Some(turn_id),
                &SessionEvent::UserMessage {
                    message: steer_msg.clone(),
                    attachments: steer.attachments,
                },
            )
            .await;
            self.core.emit_agent(AgentDomainEvent::TurnSteered {
                turn_id: turn_id.to_string(),
                conversation_id: conversation_id.to_string(),
                prompt: steer.prompt,
                trace_id: Some(trace_id.to_string()),
            });
            current_messages.push(steer_msg);
        }
        true
    }

    /// 执行一轮完整的 Agent 认知闭环
    pub async fn execute_turn(
        &self,
        conversation_id: &str,
        user_prompt: &str,
        attachments: Option<Vec<crate::agent::session::types::HistoryAttachment>>,
        cancel_token: CancellationToken,
    ) -> Result<TurnOutcome, crate::NovaCoreError> {
        let turn_id = Uuid::new_v4().to_string();
        self.execute_turn_with_id(
            conversation_id,
            &turn_id,
            user_prompt,
            attachments,
            cancel_token,
        )
        .await
    }

    /// 使用外部预分配的 `turn_id` 执行一轮完整 Agent 认知闭环（供同步仲裁回传一致 `turnId`）。
    pub async fn execute_turn_with_id(
        &self,
        conversation_id: &str,
        turn_id: &str,
        user_prompt: &str,
        attachments: Option<Vec<crate::agent::session::types::HistoryAttachment>>,
        cancel_token: CancellationToken,
    ) -> Result<TurnOutcome, crate::NovaCoreError> {
        let turn_id = turn_id.to_string();
        let trace_id = Uuid::new_v4().to_string();
        let _trace_guard = self
            .core
            .begin_turn_trace(conversation_id, &turn_id, &trace_id);
        let timestamp = chrono::Utc::now().timestamp_millis();

        tracing::info!(
            conversation_id = %conversation_id,
            turn_id = %turn_id,
            trace_id = %trace_id,
            "Agent turn started"
        );

        // 1. 发射 TurnStarted 与 AssemblingContext 领域事件
        self.core.emit_agent(AgentDomainEvent::TurnStarted {
                turn_id: turn_id.clone(),
                conversation_id: conversation_id.to_string(),
                timestamp,
                trace_id: Some(trace_id.clone()),
            },
        );
        self.core.emit_agent(AgentDomainEvent::StateChanged {
                turn_id: turn_id.clone(),
                state: CognitiveState::AssemblingContext,
                trace_id: Some(trace_id.clone()),
            },
        );

        // 2. 写入事件日志事实源（UserMessage & TurnStart）
        let user_msg = self.build_user_message(conversation_id, user_prompt, attachments.as_ref());

        let _ = crate::agent::session::append_event(
            &self.core,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::TurnStart {
                turn_id: turn_id.clone(),
                trace_id: Some(trace_id.clone()),
            },
        )
        .await;
        let _ = crate::agent::session::append_event(
            &self.core,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::UserMessage {
                message: user_msg,
                attachments: attachments.clone(),
            },
        )
        .await;

        // 3. 加载历史上下文并组装
        let raw_events = crate::agent::session::load_events(&self.core, conversation_id)
            .await
            .unwrap_or_default();
        let reconstructed =
            crate::agent::session::projection::reconstruct_model_context(&raw_events);

        let mut current_messages = match ContextAssembler::assemble(
            &self.core,
            Some(conversation_id),
            reconstructed,
        )
        .await
        {
            Ok(messages) => messages,
            Err(err) => {
                return self
                    .handle_turn_error(
                        conversation_id,
                        &turn_id,
                        &trace_id,
                        crate::NovaCoreError::Session(err),
                    )
                    .await;
            }
        };

        // 3.1 异步刷新 MCP 工具缓存（确保三协议适配器同步读取到最新的 MCP 工具清单）
        let _ = crate::agent::capabilities::mcp_tools::refresh_mcp_tools_cache(&self.core).await;

        // 4. 准备 LLM Client
        let mut client = match LlmClient::new(&self.core) {
            Ok(client) => client,
            Err(err) => {
                return self
                    .handle_turn_error(
                        conversation_id,
                        &turn_id,
                        &trace_id,
                        crate::NovaCoreError::Internal(err),
                    )
                    .await;
            }
        };

        let mut loop_count = 0;
        const MAX_STEPS: usize = 30;
        let mut total_turn_tokens = 0u32;
        let mut last_assistant_text = String::new();
        let mut all_assistant_text = String::new();
        let mut loop_breaker = ToolLoopBreaker::new();

        // 5. ReAct 核心认知循环 (Thought -> Action -> Observation -> Reaction)
        while loop_count < MAX_STEPS {
            loop_count += 1;

            if cancel_token.is_cancelled() {
                let _ = crate::agent::session::append_event(
                    &self.core,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::TurnEnd {
                        turn_id: turn_id.clone(),
                        stop_reason: Some("cancelled".to_string()),
                    },
                )
                .await;
                self.core.emit_agent(AgentDomainEvent::TurnFinished {
                        turn_id: turn_id.clone(),
                        stop_reason: "cancelled".to_string(),
                        total_tokens: total_turn_tokens,
                        trace_id: Some(trace_id.clone()),
                    },
                );
                let deliverable = if !last_assistant_text.trim().is_empty() {
                    last_assistant_text.trim().to_string()
                } else {
                    all_assistant_text.trim().to_string()
                };
                return Ok(TurnOutcome {
                    turn_id,
                    stop_reason: "cancelled".to_string(),
                    total_tokens: total_turn_tokens,
                    assistant_text: deliverable,
                });
            }

            // 5.1 消费协同 Mailbox 中的新消息与指导指令（真异步双向通信驱动）
            if crate::agent::orchestration::subagent::is_subagent_conversation(Some(conversation_id)) {
                let parent_id = crate::agent::orchestration::subagent::parent_conversation_id(conversation_id);
                if let Some(handle) = crate::agent::orchestration::registry::global_agent_registry().find(parent_id, conversation_id) {
                    let mut mb = handle.mailbox.lock().await;
                    if !mb.is_empty() {
                        let mut guidance_texts = Vec::new();
                        while let Some(msg) = mb.pop_front() {
                            guidance_texts.push(msg);
                        }
                        drop(mb);
                        let combined = guidance_texts.join("\n---\n");
                        let mailbox_text = format!(
                            "[Incoming Directive / Mailbox Message from Coordinator]:\n{}\nPlease adjust your ongoing investigation or execution accordingly.",
                            combined
                        );
                        let mailbox_msg = Message {
                            role: Role::User,
                            content: Content::Text(mailbox_text),
                        };
                        current_messages.push(mailbox_msg.clone());
                        let _ = crate::agent::session::append_event(
                            &self.core,
                            conversation_id,
                            Some(&turn_id),
                            &SessionEvent::UserMessage {
                                message: mailbox_msg,
                                attachments: None,
                            },
                        )
                        .await;
                    }
                }
            }

            // 自动上下文压缩检查 (Mid-Turn Compact)
            if let Ok(settings) = crate::services::settings::load_settings(&self.core) {
                let model = settings.active_provider_profile().model;
                let window_tokens = crate::services::settings::context_window_for_model(&settings, &model) as i64;
                let compact_outcome = crate::agent::capabilities::compact::compact_messages_mid_turn(
                    &self.core,
                    Some(conversation_id),
                    &mut current_messages,
                    window_tokens,
                )
                .await;
                if compact_outcome.applied {
                    tracing::info!(
                        turn_id = %turn_id,
                        level = compact_outcome.level,
                        tokens_before = compact_outcome.tokens_before,
                        tokens_after = compact_outcome.tokens_after,
                        "Context automatically compacted mid-turn"
                    );
                }
            }

            // 发射模型推理状态事件
            self.core.emit_agent(AgentDomainEvent::StateChanged {
                    turn_id: turn_id.clone(),
                    state: CognitiveState::ModelInference,
                    trace_id: Some(trace_id.clone()),
                },
            );

            // 发起模型流式请求（支持 prompt_too_long 自愈重试，与 Codex 一致不向请求上下文重复注入运行中后台进程列表）
            let provider_result = match client
                .send_request(
                    &self.core,
                    &current_messages,
                    AgentMode::Agent,
                    Some(conversation_id),
                )
                .await
            {
                Ok((res, _)) => res,
                Err(err) => {
                    let is_context_overflow = matches!(
                        err.error_info,
                        crate::provider::NovaErrorInfo::ContextWindowExceeded
                    ) || crate::agent::capabilities::compact::is_prompt_too_long_error(&err.message);
                    if is_context_overflow {
                        tracing::warn!(
                            turn_id = %turn_id,
                            "Prompt too long error encountered, attempting reactive compact retry"
                        );
                        if let Some(compacted) = crate::agent::capabilities::compact::reactive_compact_messages_for_retry(
                            &self.core,
                            Some(conversation_id),
                            &current_messages,
                        )
                        .await
                        {
                            current_messages = compacted;
                            match client
                                .send_request(
                                    &self.core,
                                    &current_messages,
                                    AgentMode::Agent,
                                    Some(conversation_id),
                                )
                                .await
                            {
                                Ok((res, _)) => res,
                                Err(retry_err) => {
                                    return self
                                        .handle_turn_error(
                                            conversation_id,
                                            &turn_id,
                                            &trace_id,
                                            retry_err.into(),
                                        )
                                        .await;
                                }
                            }
                        } else {
                            return self
                                .handle_turn_error(
                                    conversation_id,
                                    &turn_id,
                                    &trace_id,
                                    err.into(),
                                )
                                .await;
                        }
                    } else {
                        return self
                            .handle_turn_error(
                                conversation_id,
                                &turn_id,
                                &trace_id,
                                err.into(),
                            )
                            .await;
                    }
                }
            };

            if cancel_token.is_cancelled()
                || provider_result.stop_reason.as_deref() == Some("cancelled")
            {
                let _ = crate::agent::session::append_event(
                    &self.core,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::TurnEnd {
                        turn_id: turn_id.clone(),
                        stop_reason: Some("cancelled".to_string()),
                    },
                )
                .await;
                self.core.emit_agent(AgentDomainEvent::TurnFinished {
                        turn_id: turn_id.clone(),
                        stop_reason: "cancelled".to_string(),
                        total_tokens: total_turn_tokens,
                        trace_id: Some(trace_id.clone()),
                    },
                );
                let deliverable = if !last_assistant_text.trim().is_empty() {
                    last_assistant_text.trim().to_string()
                } else {
                    all_assistant_text.trim().to_string()
                };
                return Ok(TurnOutcome {
                    turn_id,
                    stop_reason: "cancelled".to_string(),
                    total_tokens: total_turn_tokens,
                    assistant_text: deliverable,
                });
            }

            // 发射用量事件
            let input_tok = provider_result.input_tokens.unwrap_or(0);
            let output_tok = provider_result.output_tokens.unwrap_or(0);
            let cache_read = provider_result.cache_read_tokens.unwrap_or(0);
            let cache_create = provider_result.cache_creation_tokens.unwrap_or(0);
            total_turn_tokens += input_tok + output_tok;

            self.core.emit_agent(AgentDomainEvent::TokenUsageUpdate {
                    turn_id: turn_id.clone(),
                    input_tokens: input_tok,
                    output_tokens: output_tok,
                    cache_read_tokens: cache_read,
                    cache_creation_tokens: cache_create,
                    trace_id: Some(trace_id.clone()),
                },
            );

            // 提取本轮模型生成的 assistant 纯文本
            let round_assistant_text = extract_assistant_text(&provider_result.messages);
            if !round_assistant_text.is_empty() {
                last_assistant_text = round_assistant_text.clone();
                all_assistant_text.push_str(&round_assistant_text);
                all_assistant_text.push('\n');
            }

            // 统一记账到 token_usage_log 流水表（主代理与子代理一致）
            if let Ok(settings) = crate::services::settings::load_settings(&self.core) {
                let model = settings.active_provider_profile().model;
                let provider_name = settings.provider.clone();
                let is_sub = crate::agent::orchestration::subagent::is_subagent_conversation(Some(conversation_id));
                let actual_conv_id = if is_sub {
                    crate::agent::orchestration::subagent::parent_conversation_id(conversation_id)
                } else {
                    conversation_id
                };
                let source = if is_sub { Some("subagent") } else { Some("agent") };
                let _ = crate::provider::token_usage_log::log_token_usage(
                    &self.core,
                    Some(actual_conv_id),
                    &model,
                    Some(&provider_name),
                    input_tok,
                    output_tok,
                    cache_read,
                    cache_create,
                    provider_result.cost.as_ref().map(|c| c.total_cost_usd.as_str()),
                    source,
                )
                .await;
            }

            // 将本轮模型生成的 Assistant 消息记入单事实源事件日志并加入上下文
            for msg in &provider_result.messages {
                let mut event = SessionEvent::from_model_message(msg.clone());
                if let SessionEvent::AssistantMessage {
                    ref mut token_usage,
                    ref mut cost,
                    ..
                } = event
                {
                    *token_usage = Some((input_tok + output_tok) as i64);
                    if let Some(ref c) = provider_result.cost {
                        *cost = serde_json::to_value(c).ok();
                    }
                }
                let _ = crate::agent::session::append_event(
                    &self.core,
                    conversation_id,
                    Some(&turn_id),
                    &event,
                )
                .await;
            }
            current_messages.extend(provider_result.messages);

            // 检查模型是否发起了工具调用
            let tool_calls = provider_result.tool_calls;
            if tool_calls.is_empty() || provider_result.prevent_continuation {
                if !provider_result.prevent_continuation
                    && self
                        .inject_pending_steers(
                            conversation_id,
                            &turn_id,
                            &trace_id,
                            &mut current_messages,
                        )
                        .await
                {
                    continue;
                }
                break;
            }

            // 发射工具执行状态转移事件
            self.core.emit_agent(AgentDomainEvent::StateChanged {
                    turn_id: turn_id.clone(),
                    state: CognitiveState::ExecutingTool,
                    trace_id: Some(trace_id.clone()),
                },
            );

            // 写入工具调用事件日志事实源，并向前端派发请求事件
            for call in &tool_calls {
                let _ = crate::agent::session::append_event(
                    &self.core,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::ToolCall {
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        input: serde_json::to_string(&call.input).unwrap_or_default(),
                        turn_id: Some(turn_id.clone()),
                        trace_id: Some(trace_id.clone()),
                    },
                )
                .await;

                self.core.emit_agent(AgentDomainEvent::ToolCallRequested {
                        turn_id: turn_id.clone(),
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.input.clone(),
                        trace_id: Some(trace_id.clone()),
                    },
                );
            }

            // 在 ReAct 决策循环外层执行工具（支持立即强制取消中断）
            let tool_ctx = ToolExecutionContext::new(
                self.core.clone(),
                Some(conversation_id.to_string()),
                Some(turn_id.clone()),
            )
            .with_trace_id(trace_id.clone());
            let executed_calls = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    tracing::info!("Agent turn cancelled during tool execution");
                    let _ = crate::agent::session::append_event(
                        &self.core,
                        conversation_id,
                        Some(&turn_id),
                        &SessionEvent::TurnEnd {
                            turn_id: turn_id.clone(),
                            stop_reason: Some("cancelled".to_string()),
                        },
                    )
                    .await;
                    self.core.emit_agent(AgentDomainEvent::TurnFinished {
                            turn_id: turn_id.clone(),
                            stop_reason: "cancelled".to_string(),
                            total_tokens: total_turn_tokens,
                            trace_id: Some(trace_id.clone()),
                        },
                    );
                    let deliverable = if !last_assistant_text.trim().is_empty() {
                        last_assistant_text.trim().to_string()
                    } else {
                        all_assistant_text.trim().to_string()
                    };
                    return Ok(TurnOutcome {
                        turn_id,
                        stop_reason: "cancelled".to_string(),
                        total_tokens: total_turn_tokens,
                        assistant_text: deliverable,
                    });
                }
                calls = ToolOrchestrator::run_batch(
                    &tool_ctx,
                    tool_calls.clone(),
                    &mut loop_breaker,
                ) => calls,
            };

            let mut tool_result_blocks = Vec::new();
            let mut stop_for_user_interaction = false;
            let mut pending_additional_messages: Vec<crate::provider::types::Message> = Vec::new();

            for executed in executed_calls {
                if crate::provider::stream_runner::is_needs_user_input_payload(&executed.output)
                    || executed.prevent_continuation
                {
                    stop_for_user_interaction = true;
                }

                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: executed.id,
                    is_error: executed.is_error,
                    content: vec![ContentBlock::Text {
                        text: executed.output,
                    }],
                });

                if !executed.additional_messages.is_empty() {
                    pending_additional_messages.extend(executed.additional_messages);
                }
            }

            // 封装 ToolResult 消息并以 ContextMessage 记入事件日志事实源与当前工作上下文。
            // 重要：在 Anthropic 等协议中，ToolResult 必须紧跟在包含 ToolUse 的 Assistant 消息之后！
            let tool_msg = Message {
                role: Role::User,
                content: Content::Blocks(tool_result_blocks),
            };
            let tool_event = SessionEvent::ContextMessage {
                message: tool_msg.clone(),
            };
            let _ = crate::agent::session::append_event(
                &self.core,
                conversation_id,
                Some(&turn_id),
                &tool_event,
            )
            .await;
            current_messages.push(tool_msg);

            // 挂钩或工具产生的附加上下文消息必须追加在 ToolResult 之后，严禁插在 ToolUse 与 ToolResult 之间
            if !pending_additional_messages.is_empty() {
                for add_msg in &pending_additional_messages {
                    let _ = crate::agent::session::append_event(
                        &self.core,
                        conversation_id,
                        Some(&turn_id),
                        &SessionEvent::ContextMessage {
                            message: add_msg.clone(),
                        },
                    )
                    .await;
                }
                current_messages.extend(pending_additional_messages);
            }

            if stop_for_user_interaction {
                break;
            }

            let _ = self
                .inject_pending_steers(conversation_id, &turn_id, &trace_id, &mut current_messages)
                .await;
        }

        // 6. 认知循环收尾：终结事件记录与会话元信息同步
        let final_stop_reason = if cancel_token.is_cancelled() {
            "cancelled".to_string()
        } else if loop_count >= MAX_STEPS {
            "max_steps".to_string()
        } else {
            "end_turn".to_string()
        };

        let _ = crate::agent::session::append_event(
            &self.core,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::TurnEnd {
                turn_id: turn_id.clone(),
                stop_reason: Some(final_stop_reason.clone()),
            },
        )
        .await;

        if !crate::agent::orchestration::subagent::is_subagent_conversation(Some(conversation_id)) {
            let _ = crate::agent::session::history::refresh_conversation_activity(
                &self.core,
                conversation_id,
                Some(user_prompt),
            )
            .await;
        }

        self.core.emit_agent(AgentDomainEvent::StateChanged {
                turn_id: turn_id.clone(),
                state: if cancel_token.is_cancelled() {
                    CognitiveState::Idle
                } else {
                    CognitiveState::TurnComplete
                },
                trace_id: Some(trace_id.clone()),
            },
        );

        self.core.emit_agent(AgentDomainEvent::TurnFinished {
                turn_id: turn_id.clone(),
                stop_reason: final_stop_reason.clone(),
                total_tokens: total_turn_tokens,
                trace_id: Some(trace_id.clone()),
            },
        );

        let deliverable = if !last_assistant_text.trim().is_empty() {
            last_assistant_text.trim().to_string()
        } else {
            all_assistant_text.trim().to_string()
        };

        Ok(TurnOutcome {
            turn_id,
            stop_reason: final_stop_reason,
            total_tokens: total_turn_tokens,
            assistant_text: deliverable,
        })
    }

    async fn handle_turn_error(
        &self,
        conversation_id: &str,
        turn_id: &str,
        trace_id: &str,
        error: crate::NovaCoreError,
    ) -> Result<TurnOutcome, crate::NovaCoreError> {
        let _ = crate::agent::session::append_event(
            &self.core,
            conversation_id,
            Some(turn_id),
            &SessionEvent::TurnEnd {
                turn_id: turn_id.to_string(),
                stop_reason: Some("error".to_string()),
            },
        )
        .await;
        self.core.emit_agent(AgentDomainEvent::StateChanged {
                turn_id: turn_id.to_string(),
                state: CognitiveState::Failed,
                trace_id: Some(trace_id.to_string()),
            },
        );
        self.core.emit_agent(AgentDomainEvent::TurnError {
                turn_id: turn_id.to_string(),
                error: error.to_string(),
                error_info: Some(error.to_nova_error_info()),
                trace_id: Some(trace_id.to_string()),
            },
        );
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_loop_breaker_injects_warning_at_3_failures() {
        let mut breaker = ToolLoopBreaker::new();
        let input = json!({ "path": "not_found.rs" });

        // 1st failure: None
        assert!(matches!(
            breaker.record_and_evaluate("Read", &input, true),
            CircuitAction::None
        ));

        // 2nd failure: None
        assert!(matches!(
            breaker.record_and_evaluate("Read", &input, true),
            CircuitAction::None
        ));

        // 3rd failure: InjectWarning
        match breaker.record_and_evaluate("Read", &input, true) {
            CircuitAction::InjectWarning(warn) => {
                assert!(warn.contains("failed 3 consecutive times"));
                assert!(warn.contains("Nova Circuit Breaker Warning"));
            }
            _ => panic!("Expected InjectWarning on 3rd failure"),
        }
    }

    #[test]
    fn test_tool_loop_breaker_halts_at_5_failures() {
        let mut breaker = ToolLoopBreaker::new();
        let input = json!({ "path": "not_found.rs" });

        for _ in 0..4 {
            breaker.record_and_evaluate("Read", &input, true);
        }

        // 5th failure: TripAndHalt
        match breaker.record_and_evaluate("Read", &input, true) {
            CircuitAction::TripAndHalt(halt) => {
                assert!(halt.contains("Circuit Breaker Tripped"));
                assert!(halt.contains("failed 5 consecutive times"));
            }
            _ => panic!("Expected TripAndHalt on 5th failure"),
        }
    }

    #[test]
    fn test_tool_loop_breaker_resets_on_success_or_different_params() {
        let mut breaker = ToolLoopBreaker::new();
        let input1 = json!({ "path": "file1.rs" });
        let input2 = json!({ "path": "file2.rs" });

        breaker.record_and_evaluate("Read", &input1, true);
        breaker.record_and_evaluate("Read", &input1, true);

        // Different parameter resets failure count for the new signature
        assert!(matches!(
            breaker.record_and_evaluate("Read", &input2, true),
            CircuitAction::None
        ));

        // Success resets failure count
        breaker.record_and_evaluate("Read", &input2, false);
        assert_eq!(breaker.consecutive_failure_count, 0);
    }

    #[test]
    fn test_extract_assistant_text_merges_blocks() {
        let msgs = vec![
            Message {
                role: Role::User,
                content: Content::Text("user message".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: Content::Blocks(vec![
                    ContentBlock::Text {
                        text: "Thinking done.".to_string(),
                    },
                    ContentBlock::Text {
                        text: "Here is the result.".to_string(),
                    },
                ]),
            },
        ];

        let extracted = extract_assistant_text(&msgs);
        assert!(extracted.contains("Thinking done."));
        assert!(extracted.contains("Here is the result."));
        assert!(!extracted.contains("user message"));
    }
}

