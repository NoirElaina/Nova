use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::context::assembler::ContextAssembler;
use crate::agent::events::AgentDomainEvent;
use crate::agent::state::CognitiveState;
use crate::llm::providers::LlmClient;
use crate::llm::session_log::SessionEvent;
use crate::llm::types::{AgentMode, Content, ContentBlock, Message, Role};

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
        cancel_token: CancellationToken,
    ) -> Result<(), String> {
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
        let user_msg = Message {
            role: Role::User,
            content: Content::Text(user_prompt.to_string()),
        };
        let _ = crate::llm::session_log::append_event(
            &self.app,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::TurnStart {
                turn_id: turn_id.clone(),
            },
        )
        .await;
        let _ = crate::llm::session_log::append_event(
            &self.app,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::UserMessage {
                message: user_msg,
                attachments: None,
            },
        )
        .await;

        // 3. 加载历史上下文并组装
        let raw_events = crate::llm::session_log::load_events(&self.app, conversation_id)
            .await
            .unwrap_or_default();
        let reconstructed =
            crate::llm::session_log::projection::reconstruct_model_context(&raw_events);

        let mut current_messages = ContextAssembler::assemble(
            &self.app,
            Some(conversation_id),
            reconstructed,
            user_prompt,
        )
        .await?;

        // 4. 准备 LLM Client
        let mut client = LlmClient::new(&self.app).map_err(|e| e.to_string())?;

        let mut loop_count = 0;
        const MAX_STEPS: usize = 30;
        let mut total_turn_tokens = 0u32;

        // 5. ReAct 核心认知循环 (Thought -> Action -> Observation -> Reaction)
        while loop_count < MAX_STEPS {
            loop_count += 1;

            if cancel_token.is_cancelled() {
                let _ = crate::llm::session_log::append_event(
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
                return Ok(());
            }

            // 发射模型推理状态事件
            let _ = self.app.emit(
                "agent-event",
                AgentDomainEvent::StateChanged {
                    turn_id: turn_id.clone(),
                    state: CognitiveState::ModelInference,
                },
            );

            // 发起模型流式请求
            let (provider_result, _) = match client
                .send_request(
                    &self.app,
                    &current_messages,
                    AgentMode::Agent,
                    Some(conversation_id),
                )
                .await
            {
                Ok(res) => res,
                Err(err) => {
                    let err_msg = err.message.clone();
                    let _ = crate::llm::session_log::append_event(
                        &self.app,
                        conversation_id,
                        Some(&turn_id),
                        &SessionEvent::TurnEnd {
                            turn_id: turn_id.clone(),
                            stop_reason: Some("error".to_string()),
                        },
                    )
                    .await;
                    let _ = self.app.emit(
                        "agent-event",
                        AgentDomainEvent::StateChanged {
                            turn_id: turn_id.clone(),
                            state: CognitiveState::Failed,
                        },
                    );
                    let _ = self.app.emit(
                        "agent-event",
                        AgentDomainEvent::TurnError {
                            turn_id: turn_id.clone(),
                            error: err_msg.clone(),
                        },
                    );
                    return Err(err_msg);
                }
            };

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
                let _ = crate::llm::session_log::append_event(
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
                let _ = crate::llm::session_log::append_event(
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

            // 在 ReAct 决策循环外层执行工具（沙盒、权限、自愈 AST 语法树检查）
            let exec_start = std::time::Instant::now();
            let executed_calls = crate::llm::tools::execute_tool_calls_with_app(
                &self.app,
                Some(conversation_id),
                tool_calls,
            )
            .await;
            let duration_ms = exec_start.elapsed().as_millis() as u64;

            let mut tool_result_blocks = Vec::new();
            let mut stop_for_user_interaction = false;
            let mut has_write_or_edit = false;
            let mut has_error = false;

            for executed in executed_calls {
                let name_lower = executed.name.to_lowercase();
                if name_lower == "write" || name_lower == "edit" || name_lower == "multiedit" || name_lower == "multi_edit" {
                    has_write_or_edit = true;
                }
                if executed.is_error {
                    has_error = true;
                }
                let now_ms = chrono::Utc::now().timestamp_millis();
                let _ = crate::llm::session_log::append_event(
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

                if crate::llm::providers::stream_runner::is_needs_user_input_payload(&executed.output)
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
                    current_messages.extend(executed.additional_messages);
                }
            }

            // 封装 ToolResult 消息并以 ContextMessage 记入事件日志事实源与当前工作上下文
            let tool_msg = Message {
                role: Role::User,
                content: Content::Blocks(tool_result_blocks),
            };
            let tool_event = SessionEvent::ContextMessage {
                message: tool_msg.clone(),
            };
            let _ = crate::llm::session_log::append_event(
                &self.app,
                conversation_id,
                Some(&turn_id),
                &tool_event,
            )
            .await;
            current_messages.push(tool_msg);

            if has_error {
                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::StateChanged {
                        turn_id: turn_id.clone(),
                        state: CognitiveState::Reflecting,
                    },
                );
            } else if has_write_or_edit {
                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::StateChanged {
                        turn_id: turn_id.clone(),
                        state: CognitiveState::VerifyingWorkspace,
                    },
                );
            }

            if stop_for_user_interaction {
                break;
            }
        }

        // 6. 认知循环收尾：终结事件记录与会话元信息同步
        let _ = crate::llm::session_log::append_event(
            &self.app,
            conversation_id,
            Some(&turn_id),
            &SessionEvent::TurnEnd {
                turn_id: turn_id.clone(),
                stop_reason: Some("end_turn".to_string()),
            },
        )
        .await;

        let _ = crate::llm::history::refresh_conversation_activity(
            &self.app,
            conversation_id,
            Some(user_prompt),
        )
        .await;

        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::StateChanged {
                turn_id: turn_id.clone(),
                state: CognitiveState::TurnComplete,
            },
        );

        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::TurnFinished {
                turn_id: turn_id.clone(),
                stop_reason: "end_turn".to_string(),
                total_tokens: total_turn_tokens,
            },
        );

        Ok(())
    }
}

