//! 提示词缓存击穿检测（对标 claude-code promptCacheBreakDetection 的可观测化裁剪，
//! 以及 deepseek-harness 把"前缀缓存命中"当作可测量不变量的思路）。
//!
//! 原理：三家提供商的缓存指标已统一收敛到 `cache_read` token 数
//! （Anthropic 显式 cache_control 断点 / Responses 与 OpenAI 兼容链路的服务端
//! 自动前缀缓存）。请求侧记录"系统提示词 + 工具集 + 模型 + 提供商"指纹，
//! 响应侧比较 `cache_read` 相对上一次的跌幅：跌幅 >5% 且绝对跌幅 ≥2000 token
//! 判为击穿，结合指纹变化归因后经日志 + 前端警告双通道输出。
//!
//! 基线语义：
//! - 提供商切换 = 缓存域切换，只重置基线不报击穿；
//! - compact 完成后历史整体变短，`cache_read` 自然下降，必须重置基线防误报；
//! - 会话删除时经缓存注册表调 `forget_conversation` 清理。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tracing::warn;

/// 同时跟踪的会话上限（超出按插入序驱逐最旧）。
const MAX_TRACKED_CONVERSATIONS: usize = 16;
/// 相对跌幅阈值（%）。
const DROP_PERCENT_THRESHOLD: f64 = 5.0;
/// 绝对跌幅阈值（token）：小对话的波动不值得告警。
const DROP_ABSOLUTE_THRESHOLD: u32 = 2000;

#[derive(Debug, Clone)]
struct TrackedState {
    provider_name: String,
    system_hash: u64,
    tools_hash: u64,
    model: String,
    call_count: u32,
    // 本次请求相对上一次请求的指纹变化（供击穿归因）。
    system_changed: bool,
    tools_changed: bool,
    model_changed: bool,
    provider_changed: bool,
    prev_cache_read: Option<u32>,
}

#[derive(Default)]
struct Store {
    order: VecDeque<String>,
    map: HashMap<String, TrackedState>,
}

fn store() -> &'static Mutex<Store> {
    static STATE: std::sync::OnceLock<Mutex<Store>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Store::default()))
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let digest = Sha256::digest(bytes);
    u64::from_le_bytes(digest[..8].try_into().expect("sha256 digest >= 8 bytes"))
}

fn hash_tools(tools: &[crate::llm::types::Tool]) -> u64 {
    // Vec 顺序来自 get_available_tools_for_agent（稳定注册序），序列化即确定性指纹。
    let serialized =
        serde_json::to_string(tools).unwrap_or_else(|_| format!("tool_count={}", tools.len()));
    hash_bytes(serialized.as_bytes())
}

/// 请求侧记录：刷新会话指纹并标记变化项。
///
/// 提供商切换视为缓存域切换：重置基线、清空变化标记，不参与击穿判定。
pub fn record_request(
    conversation_id: Option<&str>,
    provider_name: &str,
    system_prompt: &str,
    tools: &[crate::llm::types::Tool],
    model: &str,
) {
    let Some(conv_id) = conversation_id else {
        return;
    };
    let system_hash = hash_bytes(system_prompt.as_bytes());
    let tools_hash = hash_tools(tools);

    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    match store.map.get_mut(conv_id) {
        Some(state) => {
            state.provider_changed = state.provider_name != provider_name;
            if state.provider_changed {
                // 缓存域切换：只重置基线，历史指纹全部作废。
                state.provider_name = provider_name.to_string();
                state.system_hash = system_hash;
                state.tools_hash = tools_hash;
                state.model = model.to_string();
                state.system_changed = false;
                state.tools_changed = false;
                state.model_changed = false;
                state.call_count = 0;
                state.prev_cache_read = None;
                return;
            }
            state.system_changed = state.system_hash != system_hash;
            state.tools_changed = state.tools_hash != tools_hash;
            state.model_changed = state.model != model;
            state.system_hash = system_hash;
            state.tools_hash = tools_hash;
            state.model = model.to_string();
            state.call_count = state.call_count.saturating_add(1);
        }
        None => {
            // 容量驱逐：按插入序淘汰最旧会话。
            while store.map.len() >= MAX_TRACKED_CONVERSATIONS {
                if let Some(oldest) = store.order.pop_front() {
                    store.map.remove(&oldest);
                } else {
                    break;
                }
            }
            store.order.push_back(conv_id.to_string());
            store.map.insert(
                conv_id.to_string(),
                TrackedState {
                    provider_name: provider_name.to_string(),
                    system_hash,
                    tools_hash,
                    model: model.to_string(),
                    call_count: 1,
                    system_changed: false,
                    tools_changed: false,
                    model_changed: false,
                    provider_changed: false,
                    prev_cache_read: None,
                },
            );
        }
    }
}

/// 响应侧判定：`cache_read` 显著下跌则报击穿并归因。
///
/// 仅在上游报告了 `cache_read` 时判定（不支持缓存指标的门路自动跳过）；
/// OpenAI/DeepSeek 无 cache_creation 概念，判定只看读取量，不受影响。
/// 判定结果只写日志，不向前端弹 toast——击穿对最终用户不可操作，
/// 前端已有会话用量条的缓存命中率可观察。
pub fn check_response(
    _app: &AppHandle,
    conversation_id: Option<&str>,
    cache_read: Option<u32>,
) {
    let Some(conv_id) = conversation_id else {
        return;
    };
    let Some(read) = cache_read else {
        return;
    };

    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = store.map.get_mut(conv_id) else {
        return;
    };

    let previous = state.prev_cache_read;
    state.prev_cache_read = Some(read);
    let Some(prev) = previous else {
        // 首次响应没有基线，不判定。
        return;
    };

    // 变化标记被本次判定消费后清零，避免影响后续归因。
    let system_changed = state.system_changed;
    let tools_changed = state.tools_changed;
    let model_changed = state.model_changed;
    state.system_changed = false;
    state.tools_changed = false;
    state.model_changed = false;

    if prev == 0 || read >= prev {
        return;
    }
    let drop = prev - read;
    let drop_percent = (drop as f64 / prev as f64) * 100.0;
    if drop_percent <= DROP_PERCENT_THRESHOLD || drop < DROP_ABSOLUTE_THRESHOLD {
        return;
    }

    let mut causes: Vec<&str> = Vec::new();
    if system_changed {
        causes.push("系统提示词变化");
    }
    if tools_changed {
        causes.push("工具集变化");
    }
    if model_changed {
        causes.push("模型切换");
    }
    let cause = if causes.is_empty() {
        "提示词未变，疑似服务端缓存逐出或 TTL 过期".to_string()
    } else {
        causes.join("、")
    };
    // 缓存机制措辞：Anthropic 为显式 cache_control 断点，其余为服务端自动前缀缓存。
    // 归因逻辑一致（前缀字节不稳即击穿），仅机制描述不同。
    let mechanism = if state.provider_name.contains("anthropic") {
        "Anthropic cache_control 断点缓存"
    } else {
        "服务端自动前缀缓存"
    };
    let message = format!(
        "提示词缓存疑似击穿（{}）：cacheReadTokens {} → {}（跌幅 {:.1}%），归因：{}",
        mechanism, prev, read, drop_percent, cause
    );
    warn!(
        conversation_id = %conv_id,
        provider = %state.provider_name,
        prev_cache_read = prev,
        cache_read = read,
        cause = %cause,
        "{message}"
    );
}

/// compact 完成后重置基线：历史整体变短，`cache_read` 下降是预期而非击穿。
pub fn reset_baseline(conversation_id: Option<&str>) {
    let Some(conv_id) = conversation_id else {
        return;
    };
    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = store.map.get_mut(conv_id) {
        state.prev_cache_read = None;
        state.call_count = 0;
        state.system_changed = false;
        state.tools_changed = false;
        state.model_changed = false;
    }
}

/// 会话删除时清理跟踪状态（由缓存注册表统一调用）。
pub fn forget_conversation(conversation_id: &str) {
    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    store.map.remove(conversation_id);
    store.order.retain(|id| id != conversation_id);
}

/// 清除全部会话的跟踪状态（"清除全部会话"路径）。
pub fn clear_all() {
    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    store.map.clear();
    store.order.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(conv: &str, provider: &str, system: &str, model: &str) {
        record_request(Some(conv), provider, system, &[], model);
    }

    fn take_state(conv: &str) -> TrackedState {
        let store = store().lock().unwrap();
        store.map.get(conv).cloned().expect("state exists")
    }

    #[test]
    fn first_request_establishes_baseline_without_flags() {
        record("__pcbr_first__", "anthropic", "sys-v1", "claude-x");
        let state = take_state("__pcbr_first__");
        assert_eq!(state.call_count, 1);
        assert!(!state.system_changed);
        assert!(state.prev_cache_read.is_none());
        forget_conversation("__pcbr_first__");
    }

    #[test]
    fn unchanged_request_marks_nothing_changed() {
        record("__pcbr_same__", "anthropic", "sys-v1", "claude-x");
        record("__pcbr_same__", "anthropic", "sys-v1", "claude-x");
        let state = take_state("__pcbr_same__");
        assert!(!state.system_changed && !state.tools_changed && !state.model_changed);
        forget_conversation("__pcbr_same__");
    }

    #[test]
    fn changed_system_and_model_are_flagged() {
        record("__pcbr_diff__", "anthropic", "sys-v1", "claude-x");
        record("__pcbr_diff__", "anthropic", "sys-v2", "claude-y");
        let state = take_state("__pcbr_diff__");
        assert!(state.system_changed);
        assert!(state.model_changed);
        assert!(!state.tools_changed);
        forget_conversation("__pcbr_diff__");
    }

    #[test]
    fn provider_switch_resets_baseline_without_flags() {
        record("__pcbr_prov__", "anthropic", "sys-v1", "claude-x");
        record("__pcbr_prov__", "openai", "sys-v2", "gpt-z");
        let state = take_state("__pcbr_prov__");
        assert!(state.provider_changed);
        assert!(!state.system_changed && !state.model_changed);
        assert_eq!(state.call_count, 0);
        assert!(state.prev_cache_read.is_none());
        forget_conversation("__pcbr_prov__");
    }

    #[test]
    fn reset_baseline_drops_previous_cache_read() {
        record("__pcbr_reset__", "anthropic", "sys-v1", "claude-x");
        {
            let mut store = store().lock().unwrap();
            store.map.get_mut("__pcbr_reset__").unwrap().prev_cache_read = Some(50_000);
        }
        reset_baseline(Some("__pcbr_reset__"));
        let state = take_state("__pcbr_reset__");
        assert!(state.prev_cache_read.is_none());
        forget_conversation("__pcbr_reset__");
    }

    #[test]
    fn forget_removes_state_and_order() {
        record("__pcbr_forget__", "anthropic", "sys-v1", "claude-x");
        forget_conversation("__pcbr_forget__");
        let store = store().lock().unwrap();
        assert!(!store.map.contains_key("__pcbr_forget__"));
        assert!(!store.order.iter().any(|id| id == "__pcbr_forget__"));
    }

    #[test]
    fn eviction_keeps_capacity_bounded() {
        let ids: Vec<String> = (0..(MAX_TRACKED_CONVERSATIONS + 4))
            .map(|i| format!("__pcbr_evict_{}__", i))
            .collect();
        for id in &ids {
            record(id, "anthropic", "sys", "m");
        }
        {
            let store = store().lock().unwrap();
            assert!(store.map.len() <= MAX_TRACKED_CONVERSATIONS);
            // 最旧的应已被驱逐。
            assert!(!store.map.contains_key("__pcbr_evict_0__"));
        }
        for id in &ids {
            forget_conversation(id);
        }
    }
}
