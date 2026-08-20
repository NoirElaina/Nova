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
        map.get(&key).copied().unwrap_or(0) >= MAX_CONSECUTIVE_AUTO_COMPACT_FAILURES
    } else {
        false
    }
}

// 请求固定开销校准（C3）：真实 input（system prompt + 工具定义 + 注入上下文，
// 且天然包含分词器漂移 C4）与 messages-only 估算之间的差值。
// 压缩决策把该差值计入估算，避免"估算 85% 实际 100%"导致主动压缩永不触发。
// 无观测数据时用保守默认值：偏大只会让压缩更早触发，方向安全。
const DEFAULT_INPUT_OVERHEAD_TOKENS: i64 = 12_000;
const OVERHEAD_EMA_ALPHA: f64 = 0.3;

fn overhead_map() -> &'static Mutex<HashMap<String, i64>> {
    static OVERHEAD_MAP: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    OVERHEAD_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 记录一次真实请求的 input 开销：actual_total_input - messages_estimate。
/// EMA 平滑：system prompt / 技能 / 工具集变更后开销下调也能跟上。
pub(crate) fn record_observed_input_overhead(
    conversation_id: Option<&str>,
    actual_total_input: i64,
    messages_estimate: i64,
) {
    let Some(key) = normalize_scope_key(conversation_id) else {
        return;
    };
    if actual_total_input <= 0 || messages_estimate < 0 {
        return;
    }
    let observed = (actual_total_input - messages_estimate).max(0);
    if let Ok(mut map) = overhead_map().lock() {
        let next = match map.get(&key) {
            Some(prev) => {
                (*prev as f64 * (1.0 - OVERHEAD_EMA_ALPHA)
                    + observed as f64 * OVERHEAD_EMA_ALPHA)
                    .round() as i64
            }
            None => observed,
        };
        map.insert(key, next);
    }
}

/// 读取当前会话的 input 开销估算；无记录时返回保守默认值。
pub(crate) fn observed_input_overhead(conversation_id: Option<&str>) -> i64 {
    let Some(key) = normalize_scope_key(conversation_id) else {
        return DEFAULT_INPUT_OVERHEAD_TOKENS;
    };
    if let Ok(map) = overhead_map().lock() {
        map.get(&key)
            .copied()
            .unwrap_or(DEFAULT_INPUT_OVERHEAD_TOKENS)
    } else {
        DEFAULT_INPUT_OVERHEAD_TOKENS
    }
}
