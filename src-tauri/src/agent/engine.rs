use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::context::assembler::ContextAssembler;
use crate::agent::events::AgentDomainEvent;
use crate::agent::tools::edit::execute_exact_edit;
use crate::llm::providers::LlmClient;
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

        // 2. 加载历史上下文并组装
        let raw_events = crate::llm::session_log::load_events(&self.app, conversation_id)
            .await
            .unwrap_or_default();
        let reconstructed = crate::llm::session_log::projection::reconstruct_model_context(&raw_events);

        let mut current_messages = ContextAssembler::assemble(
            &self.app,
            Some(conversation_id),
            reconstructed,
            user_prompt,
        )
        .await?;

        // 3. 准备 LLM Client
        let mut client = LlmClient::new(&self.app).map_err(|e| e.to_string())?;

        let mut loop_count = 0;
        const MAX_STEPS: usize = 30;

        // 4. ReAct 核心循环 (Thought -> Tool Call -> Verification -> Feedback)
        while loop_count < MAX_STEPS {
            loop_count += 1;

            if cancel_token.is_cancelled() {
                let _ = self.app.emit(
                    "agent-event",
                    AgentDomainEvent::TurnFinished {
                        turn_id: turn_id.clone(),
                        stop_reason: "cancelled".to_string(),
                        total_tokens: 0,
                    },
                );
                return Ok(());
            }

            // 发起模型流式请求
            let (provider_result, _) = match client
                .send_request(&self.app, &current_messages, AgentMode::Agent, Some(conversation_id))
                .await
            {
                Ok(res) => res,
                Err(err) => {
                    let err_msg = err.message.clone();
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

            // 将本轮模型输出合并进工作上下文
            let new_messages = provider_result.messages;
            current_messages.extend(new_messages.clone());

            // 检查是否有未处理的工具调用
            let mut tool_results_to_append = Vec::new();
            for msg in &new_messages {
                if let Content::Blocks(blocks) = &msg.content {
                    for block in blocks {
                        if let ContentBlock::ToolUse { id, name, input } = block {
                            let start = Instant::now();
                            let _ = self.app.emit(
                                "agent-event",
                                AgentDomainEvent::ToolCallRequested {
                                    turn_id: turn_id.clone(),
                                    call_id: id.clone(),
                                    tool_name: name.clone(),
                                    arguments: input.clone(),
                                },
                            );

                            // 如果是 Edit 工具，直接路由到精确自愈编辑引擎
                            let (output_str, is_err) = if name == "Edit" {
                                let file_path = input.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                                let old_string = input.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                                let new_string = input.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                                let replace_all = input.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

                                let _ = self.app.emit(
                                    "agent-event",
                                    AgentDomainEvent::VerificationStarted {
                                        turn_id: turn_id.clone(),
                                        target: file_path.to_string(),
                                    },
                                );

                                match execute_exact_edit(file_path, old_string, new_string, replace_all).await {
                                    Ok(res) => {
                                        let _ = self.app.emit(
                                            "agent-event",
                                            AgentDomainEvent::VerificationCompleted {
                                                turn_id: turn_id.clone(),
                                                target: file_path.to_string(),
                                                passed: true,
                                                feedback: None,
                                            },
                                        );
                                        (res.to_string(), false)
                                    }
                                    Err(err) => {
                                        let _ = self.app.emit(
                                            "agent-event",
                                            AgentDomainEvent::VerificationCompleted {
                                                turn_id: turn_id.clone(),
                                                target: file_path.to_string(),
                                                passed: false,
                                                feedback: Some(err.clone()),
                                            },
                                        );
                                        (err, true)
                                    }
                                }
                            } else {
                                // 其它通用工具交由通用执行层处理
                                let call_req = crate::llm::tools::ToolCallRequest {
                                    id: id.clone(),
                                    name: name.clone(),
                                    input: input.clone(),
                                };
                                let res = crate::llm::tools::execute_single_tool_call(
                                    &self.app,
                                    Some(conversation_id),
                                    call_req,
                                ).await;
                                (res.output, res.is_error)
                            };

                            let duration_ms = start.elapsed().as_millis() as u64;
                            let _ = self.app.emit(
                                "agent-event",
                                AgentDomainEvent::ToolCallCompleted {
                                    turn_id: turn_id.clone(),
                                    call_id: id.clone(),
                                    tool_name: name.clone(),
                                    is_error: is_err,
                                    output: output_str.clone(),
                                    duration_ms,
                                },
                            );

                            tool_results_to_append.push(ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                is_error: is_err,
                                content: vec![ContentBlock::Text { text: output_str }],
                            });
                        }
                    }
                }
            }

            // 若本轮无工具调用，说明 Agent 任务自然完成，退出循环
            if tool_results_to_append.is_empty() {
                break;
            }

            // 将工具执行结果作为 User 角色消息回填进上下文，供模型下一步推理
            current_messages.push(Message {
                role: Role::User,
                content: Content::Blocks(tool_results_to_append),
            });
        }

        // 5. 循环结束，持久化最新轮次并通知前端
        let _ = self.app.emit(
            "agent-event",
            AgentDomainEvent::TurnFinished {
                turn_id: turn_id.clone(),
                stop_reason: "end_turn".to_string(),
                total_tokens: 0,
            },
        );

        Ok(())
    }
}
