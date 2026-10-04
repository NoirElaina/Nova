use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::context::assembler::ContextAssembler;
use crate::agent::events::AgentDomainEvent;
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

        // 1. 发射 TurnStarted 领域事件
        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::TurnStarted {
                turn_id: turn_id.clone(),
                conversation_id: conversation_id.to_string(),
                timestamp,
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

            // 将本轮模型生成的消息与工具执行结果记入单事实源事件日志
            for msg in &provider_result.messages {
                let mut event = SessionEvent::from_model_message(msg.clone());
                if let SessionEvent::AssistantMessage {
                    ref mut token_usage,
                    ..
                } = event
                {
                    *token_usage = Some((input_tok + output_tok) as i64);
                }
                let _ = crate::llm::session_log::append_event(
                    &self.app,
                    conversation_id,
                    Some(&turn_id),
                    &event,
                )
                .await;
            }

            // 检查是否有工具执行结果
            let has_tool_results = provider_result.messages.iter().any(|m| {
                matches!(
                    &m.content,
                    Content::Blocks(blocks) if blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }))
                )
            });

            // 将新消息并入工作上下文，供后续步骤参考
            current_messages.extend(provider_result.messages);

            // 若无工具调用结果或模型/钩子要求终止轮次，结束认知循环
            if !has_tool_results || provider_result.prevent_continuation {
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
            AgentDomainEvent::TurnFinished {
                turn_id: turn_id.clone(),
                stop_reason: "end_turn".to_string(),
                total_tokens: total_turn_tokens,
            },
        );

        Ok(())
    }
}

