//! 会话运行时状态机：回合并发仲裁、Steer 注入队列与顺序排队执行。

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use nova_protocol::agent::TurnSubmitMode;
use nova_protocol::session::HistoryAttachment;
use nova_protocol::sync::MutexExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct QueuedTurnInput {
    pub turn_id: String,
    pub prompt: String,
    pub attachments: Option<Vec<HistoryAttachment>>,
    pub origin_client_id: Option<String>,
}

#[derive(Debug)]
pub enum SubmitTurnDecision {
    StartNow {
        turn: QueuedTurnInput,
        cancel_token: CancellationToken,
    },
    Steered {
        turn_id: String,
        queue_length: u32,
    },
    Queued {
        turn_id: String,
        queue_length: u32,
    },
}

#[derive(Debug, Default)]
struct ConversationRuntime {
    active_turn_id: Option<String>,
    cancel_token: Option<CancellationToken>,
    origin_client_id: Option<String>,
    pending_steers: VecDeque<QueuedTurnInput>,
    queued_turns: VecDeque<QueuedTurnInput>,
}

impl ConversationRuntime {
    fn is_idle(&self) -> bool {
        self.active_turn_id.is_none()
            && self.cancel_token.is_none()
            && self.pending_steers.is_empty()
            && self.queued_turns.is_empty()
    }
}

fn runtime_state() -> &'static Mutex<HashMap<String, ConversationRuntime>> {
    static STATE: OnceLock<Mutex<HashMap<String, ConversationRuntime>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or("__default__")
        .to_string()
}

/// 同步仲裁新提交的用户回合：空闲则立即激活，忙碌则按 `mode` 写入 Steer 或顺序队列。
pub fn submit_turn(
    conversation_id: &str,
    prompt: String,
    attachments: Option<Vec<HistoryAttachment>>,
    mode: TurnSubmitMode,
    origin_client_id: Option<String>,
) -> SubmitTurnDecision {
    let key = scope_key(Some(conversation_id));
    let mut state = runtime_state().lock_unpoisoned();
    let runtime = state.entry(key).or_default();

    let active_cancelled = runtime
        .cancel_token
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled);

    match (&runtime.active_turn_id, mode, active_cancelled) {
        (None, _, _) => {
            let turn_id = uuid::Uuid::new_v4().to_string();
            let cancel_token = CancellationToken::new();
            runtime.active_turn_id = Some(turn_id.clone());
            runtime.cancel_token = Some(cancel_token.clone());
            runtime.origin_client_id = origin_client_id.clone();
            runtime.pending_steers.clear();
            SubmitTurnDecision::StartNow {
                turn: QueuedTurnInput {
                    turn_id,
                    prompt,
                    attachments,
                    origin_client_id,
                },
                cancel_token,
            }
        }
        (Some(active_id), TurnSubmitMode::StartOrSteer, false) => {
            let active_id = active_id.clone();
            runtime.pending_steers.push_back(QueuedTurnInput {
                turn_id: active_id.clone(),
                prompt,
                attachments,
                origin_client_id,
            });
            SubmitTurnDecision::Steered {
                turn_id: active_id,
                queue_length: runtime.queued_turns.len() as u32,
            }
        }
        (Some(_), _, _) => {
            let turn_id = uuid::Uuid::new_v4().to_string();
            runtime.queued_turns.push_back(QueuedTurnInput {
                turn_id: turn_id.clone(),
                prompt,
                attachments,
                origin_client_id,
            });
            SubmitTurnDecision::Queued {
                turn_id,
                queue_length: runtime.queued_turns.len() as u32,
            }
        }
    }
}

/// 在工具批次边界原子排空所有待注入的 Steer 输入。
pub fn drain_steers(conversation_id: Option<&str>) -> Vec<QueuedTurnInput> {
    let key = scope_key(conversation_id);
    let mut state = runtime_state().lock_unpoisoned();
    let Some(runtime) = state.get_mut(&key) else {
        return Vec::new();
    };
    runtime.pending_steers.drain(..).collect()
}

/// 结束当前活跃回合，并在队列非空且未被取消时原子推进到下一条排队回合。
pub fn complete_turn_and_take_next(
    conversation_id: Option<&str>,
) -> Option<(QueuedTurnInput, CancellationToken)> {
    let key = scope_key(conversation_id);
    let mut state = runtime_state().lock_unpoisoned();
    let runtime = state.get_mut(&key)?;

    let was_cancelled = runtime
        .cancel_token
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled);

    runtime.active_turn_id = None;
    runtime.cancel_token = None;
    runtime.origin_client_id = None;
    runtime.pending_steers.clear();

    if !was_cancelled {
        if let Some(next) = runtime.queued_turns.pop_front() {
            let token = CancellationToken::new();
            runtime.active_turn_id = Some(next.turn_id.clone());
            runtime.cancel_token = Some(token.clone());
            runtime.origin_client_id = next.origin_client_id.clone();
            return Some((next, token));
        }
    } else {
        runtime.queued_turns.clear();
    }

    if runtime.is_idle() {
        state.remove(&key);
    }
    None
}

/// 返回当前活跃回合的发起连接 ID（子代理自动回退到父会话绑定连接）。
pub fn active_client_id(conversation_id: Option<&str>) -> Option<String> {
    let key = scope_key(conversation_id);
    let state = runtime_state().lock_unpoisoned();
    if let Some(client_id) = state
        .get(&key)
        .and_then(|runtime| runtime.origin_client_id.clone())
    {
        return Some(client_id);
    }
    if crate::agent::orchestration::subagent::is_subagent_conversation(conversation_id) {
        let parent = crate::agent::orchestration::subagent::parent_conversation_id(&key);
        return state
            .get(parent)
            .and_then(|runtime| runtime.origin_client_id.clone());
    }
    None
}

pub fn queue_length(conversation_id: Option<&str>) -> u32 {
    let key = scope_key(conversation_id);
    let state = runtime_state().lock_unpoisoned();
    state
        .get(&key)
        .map(|runtime| runtime.queued_turns.len() as u32)
        .unwrap_or(0)
}

pub fn begin_turn(conversation_id: Option<&str>) -> bool {
    let key = scope_key(conversation_id);
    let mut state = runtime_state().lock_unpoisoned();
    let runtime = state.entry(key).or_default();
    if runtime.active_turn_id.is_some() {
        return false;
    }
    runtime.active_turn_id = Some(uuid::Uuid::new_v4().to_string());
    runtime.cancel_token = Some(CancellationToken::new());
    true
}

pub fn finish_turn(conversation_id: Option<&str>) {
    let _ = complete_turn_and_take_next(conversation_id);
}

pub fn request_cancel(conversation_id: Option<&str>) -> bool {
    let key = scope_key(conversation_id);
    let mut state = runtime_state().lock_unpoisoned();
    if let Some(runtime) = state.get_mut(&key) {
        let mut hit = false;
        if let Some(token) = runtime.cancel_token.as_ref() {
            token.cancel();
            hit = true;
        }
        if !runtime.pending_steers.is_empty() || !runtime.queued_turns.is_empty() {
            runtime.pending_steers.clear();
            runtime.queued_turns.clear();
            hit = true;
        }
        return hit;
    }
    // 兜底：若特定 key 未命中（例如前端传递 None 或切换会话），取消当前所有活跃回合。
    let mut hit = false;
    for runtime in state.values_mut() {
        if let Some(token) = runtime.cancel_token.as_ref() {
            token.cancel();
            hit = true;
        }
        runtime.pending_steers.clear();
        runtime.queued_turns.clear();
    }
    hit
}

pub fn cancel_all() -> bool {
    let mut state = runtime_state().lock_unpoisoned();
    let mut hit = false;
    for runtime in state.values_mut() {
        if let Some(token) = runtime.cancel_token.as_ref() {
            token.cancel();
            hit = true;
        }
        runtime.pending_steers.clear();
        runtime.queued_turns.clear();
    }
    hit
}

pub fn is_cancelled(conversation_id: Option<&str>) -> bool {
    let key = scope_key(conversation_id);
    let state = runtime_state().lock_unpoisoned();
    state
        .get(&key)
        .and_then(|runtime| runtime.cancel_token.as_ref())
        .is_some_and(CancellationToken::is_cancelled)
}

/// 获取指定会话的 CancellationToken 克隆，用于 async 等待取消信号。
/// 若该会话无活跃回合，返回一个永远不会被取消的令牌。
pub fn get_token(conversation_id: Option<&str>) -> CancellationToken {
    let key = scope_key(conversation_id);
    let state = runtime_state().lock_unpoisoned();
    state
        .get(&key)
        .and_then(|runtime| runtime.cancel_token.clone())
        .unwrap_or_default()
}
