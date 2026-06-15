use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

fn cancel_state() -> &'static Mutex<HashMap<String, CancellationToken>> {
    // 全局取消状态容器，只初始化一次。
    static STATE: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();
    // 若尚未初始化则创建空 HashMap 并返回全局引用。
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scope_key(conversation_id: Option<&str>) -> String {
    // 将可选会话 ID 转为规范化 key：有值用清理后的会话 ID，无值走默认 key。
    conversation_id
        // 去除会话 ID 两端空白。
        .map(str::trim)
        // 过滤掉空字符串，避免把空 ID 当作合法会话。
        .filter(|id| !id.is_empty())
        // 缺失会话 ID 时落到默认作用域。
        .unwrap_or("__default__")
        // 转成拥有所有权的 String。
        .to_string()
}

pub fn begin_turn(conversation_id: Option<&str>) {
    // 计算本轮会话作用域 key。
    let key = scope_key(conversation_id);
    // 获取全局状态锁；若锁中毒则提取内部值继续工作。
    let mut state = cancel_state().lock().unwrap_or_else(|e| e.into_inner());
    // 本轮开始时插入一个新的 CancellationToken（初始未取消）。
    state.insert(key, CancellationToken::new());
}

pub fn finish_turn(conversation_id: Option<&str>) {
    // 计算本轮会话作用域 key。
    let key = scope_key(conversation_id);
    // 获取全局状态锁；若锁中毒则提取内部值继续工作。
    let mut state = cancel_state().lock().unwrap_or_else(|e| e.into_inner());
    // 本轮结束后删除该会话的取消令牌，避免残留。
    state.remove(&key);
}

pub fn request_cancel(conversation_id: Option<&str>) -> bool {
    // 计算要取消的会话作用域 key。
    let key = scope_key(conversation_id);
    // 获取全局状态锁；若锁中毒则提取内部值继续工作。
    let state = cancel_state().lock().unwrap_or_else(|e| e.into_inner());
    // 若会话存在，触发取消令牌并返回成功。
    if let Some(token) = state.get(&key) {
        // 触发取消——所有持有该令牌克隆的 cancelled() future 立即返回。
        token.cancel();
        // 返回已成功提交取消请求。
        true
    } else {
        // 目标会话不存在，返回取消失败。
        false
    }
}

pub fn is_cancelled(conversation_id: Option<&str>) -> bool {
    // 计算当前查询的会话作用域 key。
    let key = scope_key(conversation_id);
    // 获取全局状态锁；若锁中毒则提取内部值继续工作。
    let state = cancel_state().lock().unwrap_or_else(|e| e.into_inner());
    // 读取取消令牌状态；不存在时默认 false。
    state.get(&key).map(|t| t.is_cancelled()).unwrap_or(false)
}

/// 获取指定会话的 CancellationToken 克隆，用于 async 等待取消信号。
/// 若该会话无活跃回合，返回一个永远不会被取消的令牌。
pub fn get_token(conversation_id: Option<&str>) -> CancellationToken {
    let key = scope_key(conversation_id);
    let state = cancel_state().lock().unwrap_or_else(|e| e.into_inner());
    state.get(&key).cloned().unwrap_or_default()
}
