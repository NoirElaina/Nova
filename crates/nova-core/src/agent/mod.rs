pub mod cancellation;
pub mod capabilities;
pub mod context;
pub mod engine;
pub mod events;
pub mod lifecycle;
pub mod orchestration;
pub mod permissions;
pub mod sandbox;
pub mod session;
pub mod session_runtime;
pub mod state;
pub mod tools;
pub mod utils;

use nova_protocol::agent::{
    AgentTurnSubmitOutcome, TurnSubmitMode, TurnSubmitOutcomeKind,
};
use tokio_util::sync::CancellationToken;

use crate::agent::session::types::HistoryAttachment;
use crate::core::Core;

/// 同步提交仲裁产物：`dispatch` 在 `tokio::spawn` 之前完成并发、Steer 与队列裁决。
pub struct PreparedTurnLaunch {
    pub outcome: AgentTurnSubmitOutcome,
    start: Option<(session_runtime::QueuedTurnInput, CancellationToken)>,
}

/// 同步仲裁回合提交；仅当返回需要立即启动执行链时才需要由调用方 `tokio::spawn`。
pub fn prepare_modern_agent_turn(
    core: &Core,
    conversation_id: &str,
    prompt: &str,
    attachments: Option<Vec<HistoryAttachment>>,
    mode: TurnSubmitMode,
    origin_client_id: Option<String>,
) -> Result<PreparedTurnLaunch, String> {
    let trimmed_id = conversation_id.trim();
    if trimmed_id.is_empty() {
        return Err("conversationId 不能为空。".to_string());
    }
    let has_attachments = attachments.as_ref().is_some_and(|list| !list.is_empty());
    if prompt.trim().is_empty() && !has_attachments {
        return Err("消息内容与附件不能同时为空。".to_string());
    }

    match session_runtime::submit_turn(
        trimmed_id,
        prompt.to_string(),
        attachments,
        mode,
        origin_client_id,
    ) {
        session_runtime::SubmitTurnDecision::StartNow { turn, cancel_token } => {
            Ok(PreparedTurnLaunch {
                outcome: AgentTurnSubmitOutcome {
                    outcome: TurnSubmitOutcomeKind::Started,
                    turn_id: turn.turn_id.clone(),
                    queue_length: 0,
                },
                start: Some((turn, cancel_token)),
            })
        }
        session_runtime::SubmitTurnDecision::Steered {
            turn_id,
            queue_length,
        } => Ok(PreparedTurnLaunch {
            outcome: AgentTurnSubmitOutcome {
                outcome: TurnSubmitOutcomeKind::Steered,
                turn_id,
                queue_length,
            },
            start: None,
        }),
        session_runtime::SubmitTurnDecision::Queued {
            turn_id,
            queue_length,
        } => {
            core.emit_agent(crate::agent::events::AgentDomainEvent::TurnQueued {
                turn_id: turn_id.clone(),
                conversation_id: trimmed_id.to_string(),
                queue_length,
                trace_id: None,
            });
            Ok(PreparedTurnLaunch {
                outcome: AgentTurnSubmitOutcome {
                    outcome: TurnSubmitOutcomeKind::Queued,
                    turn_id,
                    queue_length,
                },
                start: None,
            })
        }
    }
}

struct TurnSlotGuard(Option<String>);

impl Drop for TurnSlotGuard {
    fn drop(&mut self) {
        if let Some(conv_id) = self.0.take() {
            session_runtime::abort_turn_slot(Some(&conv_id));
        }
    }
}

/// 顺序执行已同步激活的回合，并在回合正常完成后自动消费同会话队列中的后续回合。
pub async fn run_prepared_turn_sequence(
    core: Core,
    conversation_id: String,
    launch: PreparedTurnLaunch,
) -> Result<(), String> {
    let Some(initial) = launch.start else {
        return Ok(());
    };
    let mut current = Some(initial);
    let mut first_error: Option<String> = None;
    let mut slot_guard = TurnSlotGuard(Some(conversation_id.clone()));

    while let Some((entry, cancel_token)) = current {
        let engine = engine::AgentEngine::new(core.clone());
        let result = engine
            .execute_turn_with_id(
                &conversation_id,
                &entry.turn_id,
                &entry.prompt,
                entry.attachments,
                cancel_token,
            )
            .await;
        if let Err(error) = result {
            if first_error.is_none() {
                first_error = Some(error.into());
            }
        }
        current = session_runtime::complete_turn_and_take_next(Some(&conversation_id));
    }

    slot_guard.0 = None;
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 现代化 Agent 外部统一调用入口（阻塞等待执行完成；无活跃回合时直接执行，有活跃回合则按队列/Steer仲裁）。
pub async fn send_modern_agent_turn(
    core: Core,
    conversation_id: String,
    prompt: String,
    attachments: Option<Vec<HistoryAttachment>>,
) -> Result<(), String> {
    let launch = prepare_modern_agent_turn(
        &core,
        &conversation_id,
        &prompt,
        attachments,
        TurnSubmitMode::StartOrSteer,
        None,
    )?;
    run_prepared_turn_sequence(core, conversation_id, launch).await
}

/// 现代化 Agent 外部取消统一调用入口
pub async fn cancel_modern_agent_turn(conversation_id: Option<String>) -> Result<bool, String> {
    Ok(session_runtime::request_cancel(conversation_id.as_deref()))
}
