use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTurnStatus {
    pub conversation_id: String,
    pub state: String,
    pub assistant_response: String,
    pub assistant_reasoning: String,
    pub started_at: u64,
    pub updated_at: u64,
}

fn live_turns() -> &'static Mutex<HashMap<String, LiveTurnStatus>> {
    static STATE: OnceLock<Mutex<HashMap<String, LiveTurnStatus>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn scope_key(conversation_id: Option<&str>) -> Option<String> {
    conversation_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub fn begin_turn(conversation_id: Option<&str>) {
    let Some(key) = scope_key(conversation_id) else {
        return;
    };
    let now = now_millis();
    let mut state = live_turns()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state.insert(
        key.clone(),
        LiveTurnStatus {
            conversation_id: key,
            state: "running".to_string(),
            assistant_response: String::new(),
            assistant_reasoning: String::new(),
            started_at: now,
            updated_at: now,
        },
    );
}

pub fn append_text(conversation_id: Option<&str>, text: &str) {
    let Some(key) = scope_key(conversation_id) else {
        return;
    };
    let mut state = live_turns()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(turn) = state.get_mut(&key) {
        turn.assistant_response.push_str(text);
        turn.updated_at = now_millis();
    }
}

pub fn append_reasoning(conversation_id: Option<&str>, text: &str) {
    let Some(key) = scope_key(conversation_id) else {
        return;
    };
    let mut state = live_turns()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(turn) = state.get_mut(&key) {
        turn.assistant_reasoning.push_str(text);
        turn.updated_at = now_millis();
    }
}

pub fn mark_terminal(conversation_id: Option<&str>, terminal_state: &str) {
    let Some(key) = scope_key(conversation_id) else {
        return;
    };
    let mut state = live_turns()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(turn) = state.get_mut(&key) {
        turn.state = terminal_state.to_string();
        turn.updated_at = now_millis();
    }
}

pub fn get_status(conversation_id: Option<&str>) -> Option<LiveTurnStatus> {
    let key = scope_key(conversation_id)?;
    live_turns()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .cloned()
}

pub fn ack_status(conversation_id: Option<&str>) -> bool {
    let Some(key) = scope_key(conversation_id) else {
        return false;
    };
    live_turns()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&key)
        .is_some()
}
