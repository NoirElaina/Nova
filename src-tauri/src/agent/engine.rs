use tauri::{AppHandle, Emitter};
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

/// 工具调用循环熔断动作
enum CircuitAction {
    None,
    InjectWarning(String),
    TripAndHalt(String),
}

/// 工具调用死循环熔断器
/// 针对连续相同参数且执行报错/无意义重复的调用进行即时警示或截断，杜绝无脑试错 30 步烧穿 token
#[derive(Default)]
struct ToolLoopBreaker {
    last_signature: Option<String>,
    consecutive_same_call_count: usize,
    consecutive_failure_count: usize,
}

impl ToolLoopBreaker {
    fn new() -> Self {
        Self::default()
    }

    fn signature(name: &str, input: &serde_json::Value) -> String {
        format!("{}:{}", name, input)
    }

    fn record_and_evaluate(
        &mut self,
        tool_name: &str,
        input: &serde_json::Value,
        is_error: bool,
    ) -> CircuitAction {
        let sig = Self::signature(tool_name, input);
        if self.last_signature.as_deref() == Some(&sig) {
            self.consecutive_same_call_count += 1;
            if is_error {
                self.consecutive_failure_count += 1;
            } else {
                self.consecutive_failure_count = 0;
            }
        } else {
            self.last_signature = Some(sig);
            self.consecutive_same_call_count = 1;
            self.consecutive_failure_count = if is_error { 1 } else { 0 };
        }

        if self.consecutive_failure_count >= 5 {
            CircuitAction::TripAndHalt(format!(
                "[Circuit Breaker Tripped]: Tool '{}' has failed 5 consecutive times with identical parameters. Halting execution to prevent infinite loop.",
                tool_name
            ))
        } else if self.consecutive_failure_count >= 3 {
            CircuitAction::InjectWarning(format!(
                "\n\n[Nova Circuit Breaker Warning]: Tool '{}' has failed {} consecutive times with the exact same input. You MUST try a different approach, verify path/arguments, or explain the difficulty to the user instead of retrying this call.",
                tool_name, self.consecutive_failure_count
            ))
        } else if self.consecutive_same_call_count >= 6 {
            CircuitAction::InjectWarning(format!(
                "\n\n[Nova Circuit Breaker Warning]: Tool '{}' has been invoked {} consecutive times with identical parameters. Please progress to the next step or synthesize results.",
                tool_name, self.consecutive_same_call_count
            ))
        } else {
            CircuitAction::None
        }
    }
}

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
    app: AppHandle,
}

impl AgentEngine {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// 执行一轮完整的 Agent 认知闭环
    pub async fn execute_turn(
        &self,
        conversation_id: &str,
        user_prompt: &str,
        attachments: Option<Vec<crate::agent::session::types::HistoryAttachment>>,
        cancel_token: CancellationToken,
    ) -> Result<TurnOutcome, String> {
        let turn_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().timestamp_millis();

        // 1. 发射 TurnStarted 与 AssemblingContext 领域事件
        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::TurnStarted {
                turn_id: turn_id.clone(),
                conversation_id: conversation_id.to_string(),
                timestamp,
            },
        );
        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::StateChanged {
                turn_id: turn_id.clone(),
                state: CognitiveState::AssemblingContext,
            },
        );

        // 2. 写入事件日志事实源（UserMessage & TurnStart）
        let mut text_content = user_prompt.to_string();
        let mut content_blocks = Vec::new();

        if let Some(ref atts) = attachments {
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
                            if let Ok(extracted) = crate::agent::capabilities::session_files::read_session_file(
                                &self.app,
                                conversation_id,
                                file_path,
                            ) {
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

        let user_msg = if content_blocks.is_empty() {
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
        };

        let _ = crate::agent::session::append_event(
            &self.app,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::TurnStart {
                turn_id: turn_id.clone(),
            },
        )
        .await;
        let _ = crate::agent::session::append_event(
            &self.app,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::UserMessage {
                message: user_msg,
                attachments: attachments.clone(),
            },
        )
        .await;

        // 3. 加载历史上下文并组装
        let raw_events = crate::agent::session::load_events(&self.app, conversation_id)
            .await
            .unwrap_or_default();
        let reconstructed =
            crate::agent::session::projection::reconstruct_model_context(&raw_events);

        let mut current_messages = ContextAssembler::assemble(
            &self.app,
            Some(conversation_id),
            reconstructed,
        )
        .await?;

        // 4. 准备 LLM Client
        let mut client = LlmClient::new(&self.app).map_err(|e| e.to_string())?;

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
                    &self.app,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::TurnEnd {
                        turn_id: turn_id.clone(),
                        stop_reason: Some("cancelled".to_string()),
                    },
                )
                .await;
                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::TurnFinished {
                        turn_id: turn_id.clone(),
                        stop_reason: "cancelled".to_string(),
                        total_tokens: total_turn_tokens,
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

            // 自动上下文压缩检查 (Mid-Turn Compact)
            if let Ok(settings) = crate::services::settings::load_settings(&self.app) {
                let model = settings.active_provider_profile().model;
                let window_tokens = settings.context_window_for_model(&model) as i64;
                let compact_outcome = crate::agent::capabilities::compact::compact_messages_mid_turn(
                    &self.app,
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
            let _ = self.app.emit(
                "agent-event",
                AgentDomainEvent::StateChanged {
                    turn_id: turn_id.clone(),
                    state: CognitiveState::ModelInference,
                },
            );

            // 发起模型流式请求（支持 prompt_too_long 自愈重试）
            let provider_result = match client
                .send_request(
                    &self.app,
                    &current_messages,
                    AgentMode::Agent,
                    Some(conversation_id),
                )
                .await
            {
                Ok((res, _)) => res,
                Err(err) => {
                    let err_msg = err.message.clone();
                    if crate::agent::capabilities::compact::is_prompt_too_long_error(&err_msg) {
                        tracing::warn!(
                            turn_id = %turn_id,
                            "Prompt too long error encountered, attempting reactive compact retry"
                        );
                        if let Some(compacted) = crate::agent::capabilities::compact::reactive_compact_messages_for_retry(
                            &self.app,
                            Some(conversation_id),
                            &current_messages,
                        )
                        .await
                        {
                            current_messages = compacted;
                            match client
                                .send_request(
                                    &self.app,
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
                                            &retry_err.message,
                                        )
                                        .await;
                                }
                            }
                        } else {
                            return self
                                .handle_turn_error(conversation_id, &turn_id, &err_msg)
                                .await;
                        }
                    } else {
                        return self
                            .handle_turn_error(conversation_id, &turn_id, &err_msg)
                            .await;
                    }
                }
            };

            if cancel_token.is_cancelled()
                || provider_result.stop_reason.as_deref() == Some("cancelled")
            {
                let _ = crate::agent::session::append_event(
                    &self.app,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::TurnEnd {
                        turn_id: turn_id.clone(),
                        stop_reason: Some("cancelled".to_string()),
                    },
                )
                .await;
                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::TurnFinished {
                        turn_id: turn_id.clone(),
                        stop_reason: "cancelled".to_string(),
                        total_tokens: total_turn_tokens,
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

            let _ = self.app.emit(
                "agent-event",
                AgentDomainEvent::TokenUsageUpdate {
                    turn_id: turn_id.clone(),
                    input_tokens: input_tok,
                    output_tokens: output_tok,
                    cache_read_tokens: cache_read,
                    cache_creation_tokens: cache_create,
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
            if let Ok(settings) = crate::services::settings::load_settings(&self.app) {
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
                    &self.app,
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
                    &self.app,
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
                break;
            }

            // 发射工具执行状态转移事件
            let _ = self.app.emit(
                "agent-event",
                AgentDomainEvent::StateChanged {
                    turn_id: turn_id.clone(),
                    state: CognitiveState::ExecutingTool,
                },
            );

            // 写入工具调用事件日志事实源，并向前端派发请求事件
            for call in &tool_calls {
                let _ = crate::agent::session::append_event(
                    &self.app,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::ToolCall {
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        input: serde_json::to_string(&call.input).unwrap_or_default(),
                        turn_id: Some(turn_id.clone()),
                    },
                )
                .await;

                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::ToolCallRequested {
                        turn_id: turn_id.clone(),
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.input.clone(),
                    },
                );
            }

            // 在 ReAct 决策循环外层执行工具（支持立即强制取消中断）
            let exec_start = std::time::Instant::now();
            let executed_calls = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    tracing::info!("Agent turn cancelled during tool execution");
                    let _ = crate::agent::session::append_event(
                        &self.app,
                        conversation_id,
                        Some(&turn_id),
                        &SessionEvent::TurnEnd {
                            turn_id: turn_id.clone(),
                            stop_reason: Some("cancelled".to_string()),
                        },
                    )
                    .await;
                    let _ = self.app.emit(
                        "agent-event",
                        AgentDomainEvent::TurnFinished {
                            turn_id: turn_id.clone(),
                            stop_reason: "cancelled".to_string(),
                            total_tokens: total_turn_tokens,
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
                calls = crate::agent::tools::execute_tool_calls_with_app(
                    &self.app,
                    Some(conversation_id),
                    tool_calls.clone(),
                ) => calls,
            };
            let duration_ms = exec_start.elapsed().as_millis() as u64;

            let mut tool_result_blocks = Vec::new();
            let mut stop_for_user_interaction = false;
            let mut pending_additional_messages = Vec::new();

            for mut executed in executed_calls {
                // 提取本次工具调用原始入参以计算特征指纹
                let call_input = tool_calls
                    .iter()
                    .find(|c| c.id == executed.id)
                    .map(|c| c.input.clone())
                    .unwrap_or_default();

                // 接入工具死循环熔断器
                match loop_breaker.record_and_evaluate(&executed.name, &call_input, executed.is_error) {
                    CircuitAction::None => {}
                    CircuitAction::InjectWarning(warning) => {
                        tracing::warn!(
                            tool = %executed.name,
                            "ToolLoopBreaker injected warning for repeated execution"
                        );
                        executed.output.push_str(&warning);
                    }
                    CircuitAction::TripAndHalt(halt_msg) => {
                        tracing::error!(
                            tool = %executed.name,
                            "ToolLoopBreaker tripped and halted execution"
                        );
                        executed.output = halt_msg;
                        executed.is_error = true;
                        stop_for_user_interaction = true;
                    }
                }

                let now_ms = chrono::Utc::now().timestamp_millis();
                let _ = crate::agent::session::append_event(
                    &self.app,
                    conversation_id,
                    Some(&turn_id),
                    &SessionEvent::ToolResult {
                        call_id: executed.id.clone(),
                        tool_name: executed.name.clone(),
                        output: executed.output.clone(),
                        is_error: executed.is_error,
                        started_at: now_ms.saturating_sub(duration_ms as i64),
                        finished_at: now_ms,
                    },
                )
                .await;

                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::ToolCallCompleted {
                        turn_id: turn_id.clone(),
                        call_id: executed.id.clone(),
                        tool_name: executed.name.clone(),
                        is_error: executed.is_error,
                        output: executed.output.clone(),
                        duration_ms,
                    },
                );

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
                &self.app,
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
                        &self.app,
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
            &self.app,
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
                &self.app,
                conversation_id,
                Some(user_prompt),
            )
            .await;
        }

        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::StateChanged {
                turn_id: turn_id.clone(),
                state: if cancel_token.is_cancelled() {
                    CognitiveState::Idle
                } else {
                    CognitiveState::TurnComplete
                },
            },
        );

        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::TurnFinished {
                turn_id: turn_id.clone(),
                stop_reason: final_stop_reason.clone(),
                total_tokens: total_turn_tokens,
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
        err_msg: &str,
    ) -> Result<TurnOutcome, String> {
        let _ = crate::agent::session::append_event(
            &self.app,
            conversation_id,
            Some(turn_id),
            &SessionEvent::TurnEnd {
                turn_id: turn_id.to_string(),
                stop_reason: Some("error".to_string()),
            },
        )
        .await;
        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::StateChanged {
                turn_id: turn_id.to_string(),
                state: CognitiveState::Failed,
            },
        );
        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::TurnError {
                turn_id: turn_id.to_string(),
                error: err_msg.to_string(),
            },
        );
        Err(err_msg.to_string())
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

