use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const MAX_CONSECUTIVE_AUTO_COMPACT_FAILURES: u8 = 3;

fn failure_map() -> &'static Mutex<HashMap<String, u8>> {
    static FAILURE_MAP: OnceLock<Mutex<HashMap<String, u8>>> = OnceLock::new();
    FAILURE_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_scope_key(conversation_id: Option<&str>) -> Option<String> {
    conversation_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub(crate) fn record_auto_compact_success(conversation_id: Option<&str>) {
    let Some(key) = normalize_scope_key(conversation_id) else {
        return;
    };

    if let Ok(mut map) = failure_map().lock() {
        map.remove(&key);
    }
}

pub(crate) fn record_auto_compact_failure(conversation_id: Option<&str>) -> u8 {
    let Some(key) = normalize_scope_key(conversation_id) else {
        return 0;
    };

    if let Ok(mut map) = failure_map().lock() {
        let entry = map.entry(key).or_insert(0);
        *entry = entry.saturating_add(1);
        *entry
    } else {
        0
    }
}

pub(crate) fn is_auto_compact_circuit_open(conversation_id: Option<&str>) -> bool {
    let Some(key) = normalize_scope_key(conversation_id) else {
        return false;
    };

    if let Ok(map) = failure_map().lock() {
        map.get(&key)
            .copied()
            .unwrap_or(0)
            >= MAX_CONSECUTIVE_AUTO_COMPACT_FAILURES
    } else {
        false
    }
}
