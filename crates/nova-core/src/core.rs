//! 内核唯一上下文：配置 + 事件枢纽 + 宿主能力。
//!
//! 所有领域模块（引擎/工具/权限/会话/Provider）只依赖 `Core`，
//! 不感知 CLI、守护进程、桌面端等任何宿主形态。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::config::CoreConfig;
use crate::events::{EventHub, NovaEvent};

/// 内存环形缓冲区保留的最近脱敏 wire 请求/响应条数上限。
pub const WIRE_RING_BUFFER_CAPACITY: usize = 32;
const WIRE_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireDirection {
    Request,
    Response,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedWireRecord {
    pub timestamp_ms: i64,
    pub trace_id: Option<String>,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub direction: WireDirection,
    pub url: Option<String>,
    pub status: Option<u16>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
struct ActiveTurnTrace {
    turn_id: String,
    trace_id: String,
}

#[derive(Default)]
struct TraceRegistry {
    by_conversation: HashMap<String, ActiveTurnTrace>,
    by_turn: HashMap<String, String>,
}

/// RAII 作用域守卫：Turn 退出/取消/panic 时自动清理活跃 trace 映射。
pub struct ActiveTraceGuard {
    core: Core,
    conversation_id: String,
    turn_id: String,
}

impl Drop for ActiveTraceGuard {
    fn drop(&mut self) {
        self.core.end_turn_trace(&self.conversation_id, &self.turn_id);
    }
}

/// 桌面浏览器自动化能力（宿主注入；无头环境缺省，浏览器工具如实报不可用）。
#[async_trait]
pub trait BrowserCapability: Send + Sync {
    async fn run_command(
        &self,
        conversation_id: Option<&str>,
        action: &str,
        payload: serde_json::Value,
        timeout_ms: Option<u64>,
    ) -> serde_json::Value;
}

/// 宿主生命周期钩子（用户终端清理、文件日志开关；缺省空实现）。
pub trait HostHooks: Send + Sync {
    fn stop_user_terminal_session(&self, _conversation_id: &str) {}
    fn close_all_user_terminal_sessions(&self) {}
    fn set_file_logging_enabled(&self, _enabled: bool) {}
}

pub struct NoopHostHooks;
impl HostHooks for NoopHostHooks {}

struct CoreInner {
    config: CoreConfig,
    events: EventHub,
    browser: RwLock<Option<Arc<dyn BrowserCapability>>>,
    hooks: RwLock<Arc<dyn HostHooks>>,
    traces: RwLock<TraceRegistry>,
    wire_traces: RwLock<VecDeque<RedactedWireRecord>>,
    idempotency: crate::idempotency::IdempotencyCache,
    db_pool: tokio::sync::OnceCell<sqlx::SqlitePool>,
}

/// 内核上下文句柄（Arc 共享，廉价克隆，线程安全）。
#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
}

impl Core {
    pub fn new(config: CoreConfig, event_capacity: usize) -> Self {
        // 协同看板需要数据目录做落盘（跨重启保留频道与帖子）。
        crate::agent::orchestration::board::init_storage(&config.data_dir());
        Self {
            inner: Arc::new(CoreInner {
                config,
                events: EventHub::new(event_capacity),
                browser: RwLock::new(None),
                hooks: RwLock::new(Arc::new(NoopHostHooks)),
                traces: RwLock::new(TraceRegistry::default()),
                wire_traces: RwLock::new(VecDeque::with_capacity(WIRE_RING_BUFFER_CAPACITY)),
                idempotency: crate::idempotency::IdempotencyCache::default(),
                db_pool: tokio::sync::OnceCell::const_new(),
            }),
        }
    }

    pub async fn run_idempotent<T, E, F, Fut>(
        &self,
        scope: &str,
        key: Option<&str>,
        op: F,
    ) -> Result<T, E>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        self.inner.idempotency.run(scope, key, op).await
    }

    pub(crate) fn db_pool_cell(&self) -> &tokio::sync::OnceCell<sqlx::SqlitePool> {
        &self.inner.db_pool
    }

    /// 注册当前会话与回合的活跃 `trace_id`，返回离开作用域即自动撤销的守卫。
    pub fn begin_turn_trace(
        &self,
        conversation_id: &str,
        turn_id: &str,
        trace_id: &str,
    ) -> ActiveTraceGuard {
        if let Ok(mut guard) = self.inner.traces.write() {
            guard.by_conversation.insert(
                conversation_id.to_string(),
                ActiveTurnTrace {
                    turn_id: turn_id.to_string(),
                    trace_id: trace_id.to_string(),
                },
            );
            guard
                .by_turn
                .insert(turn_id.to_string(), trace_id.to_string());
        }
        ActiveTraceGuard {
            core: self.clone(),
            conversation_id: conversation_id.to_string(),
            turn_id: turn_id.to_string(),
        }
    }

    fn end_turn_trace(&self, conversation_id: &str, turn_id: &str) {
        if let Ok(mut guard) = self.inner.traces.write() {
            guard.by_turn.remove(turn_id);
            let matches_conv = guard
                .by_conversation
                .get(conversation_id)
                .is_some_and(|entry| entry.turn_id == turn_id);
            if matches_conv {
                guard.by_conversation.remove(conversation_id);
            }
        }
    }

    /// 按 conversation_id 或 turn_id 解析活跃的 `(turn_id, trace_id)`。
    pub fn active_trace_for_conversation(&self, conversation_id: &str) -> Option<(String, String)> {
        self.inner
            .traces
            .read()
            .ok()
            .and_then(|guard| {
                guard
                    .by_conversation
                    .get(conversation_id)
                    .map(|entry| (entry.turn_id.clone(), entry.trace_id.clone()))
            })
    }

    pub fn active_trace_id(&self, conversation_id: Option<&str>, turn_id: Option<&str>) -> Option<String> {
        let guard = self.inner.traces.read().ok()?;
        if let Some(tid) = turn_id {
            if let Some(trace_id) = guard.by_turn.get(tid) {
                return Some(trace_id.clone());
            }
        }
        if let Some(cid) = conversation_id {
            if let Some(entry) = guard.by_conversation.get(cid) {
                return Some(entry.trace_id.clone());
            }
        }
        None
    }

    /// 记录最近 N 次已脱敏 wire 请求/响应到内存环形缓冲区（复用 `nova_protocol::redact`）。
    pub fn record_redacted_wire(&self, mut record: RedactedWireRecord) {
        if record.trace_id.is_none() {
            record.trace_id = self.active_trace_id(
                record.conversation_id.as_deref(),
                record.turn_id.as_deref(),
            );
        }
        record.url = record
            .url
            .map(|raw| nova_protocol::redact::redact_secrets(&raw));
        let mut scrubbed = nova_protocol::redact::redact_json_value(&record.payload);
        if let Ok(serialized) = serde_json::to_string(&scrubbed) {
            if serialized.len() > WIRE_PAYLOAD_MAX_BYTES {
                let clipped =
                    nova_protocol::redact::truncate_utf8(&serialized, WIRE_PAYLOAD_MAX_BYTES);
                scrubbed = serde_json::json!({
                    "truncated": true,
                    "originalBytes": serialized.len(),
                    "preview": clipped,
                });
            }
        }
        record.payload = scrubbed;

        if let Ok(mut guard) = self.inner.wire_traces.write() {
            if guard.len() >= WIRE_RING_BUFFER_CAPACITY {
                guard.pop_front();
            }
            guard.push_back(record);
        }
    }

    pub fn recent_wire_traces(&self) -> Vec<RedactedWireRecord> {
        self.inner
            .wire_traces
            .read()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn last_wire_trace(&self) -> Option<RedactedWireRecord> {
        self.inner
            .wire_traces
            .read()
            .ok()
            .and_then(|guard| guard.back().cloned())
    }

    pub fn with_browser(self, capability: Arc<dyn BrowserCapability>) -> Self {
        if let Ok(mut guard) = self.inner.browser.write() {
            *guard = Some(capability);
        }
        self
    }

    pub fn with_hooks(self, hooks: Arc<dyn HostHooks>) -> Self {
        if let Ok(mut guard) = self.inner.hooks.write() {
            *guard = hooks;
        }
        self
    }

    pub fn config(&self) -> &CoreConfig {
        &self.inner.config
    }

    pub fn events(&self) -> &EventHub {
        &self.inner.events
    }

    /// 发射一条内核领域事件。
    pub fn emit(&self, event: NovaEvent) {
        self.inner.events.emit(event);
    }

    /// Agent 领域事件快捷出口（最高频事件类型），自动补全活跃回合的 trace_id。
    pub fn emit_agent(&self, mut event: crate::agent::events::AgentDomainEvent) {
        if event.trace_id().is_none() {
            let tid = event.turn_id().to_string();
            let conv_hint = event.conversation_id_hint().map(str::to_string);
            let resolved = self
                .active_trace_id(conv_hint.as_deref(), Some(&tid))
                .or_else(|| self.active_trace_id(Some(&tid), None));
            if let Some(trace_id) = resolved {
                event.set_trace_id(trace_id);
            }
        }
        self.emit(NovaEvent::Agent(event));
    }

    pub fn browser(&self) -> Option<Arc<dyn BrowserCapability>> {
        self.inner.browser.read().ok().and_then(|guard| guard.clone())
    }

    pub fn hooks(&self) -> Arc<dyn HostHooks> {
        self.inner
            .hooks
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| Arc::new(NoopHostHooks))
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<NovaEvent> {
        self.inner.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_ring_buffer_caps_capacity_and_redacts_secrets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let core = Core::new(CoreConfig::new(dir.path().to_path_buf()), 32);

        let guard = core.begin_turn_trace("conv-1", "turn-1", "trace-abc");
        for i in 0..(WIRE_RING_BUFFER_CAPACITY + 10) {
            core.record_redacted_wire(RedactedWireRecord {
                timestamp_ms: i as i64,
                trace_id: None,
                conversation_id: Some("conv-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                direction: WireDirection::Request,
                url: Some(format!("https://u:secretpass@api.example.com/v1?seq={}", i)),
                status: None,
                payload: serde_json::json!({
                    "seq": i,
                    "api_key": "sk-proj-supersecret12345",
                    "prompt": "ok"
                }),
            });
        }
        drop(guard);

        let records = core.recent_wire_traces();
        assert_eq!(records.len(), WIRE_RING_BUFFER_CAPACITY);
        assert_eq!(records[0].payload["seq"], 10);
        let last = core.last_wire_trace().expect("last wire trace");
        assert_eq!(last.payload["seq"], WIRE_RING_BUFFER_CAPACITY + 9);
        assert_eq!(last.trace_id.as_deref(), Some("trace-abc"));
        let serialized = serde_json::to_string(&last).expect("serialize");
        assert!(!serialized.contains("secretpass"));
        assert!(!serialized.contains("sk-proj-supersecret12345"));
    }

    #[test]
    fn emit_agent_auto_populates_and_clears_trace_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let core = Core::new(CoreConfig::new(dir.path().to_path_buf()), 32);
        let mut rx = core.subscribe();

        let guard = core.begin_turn_trace("conv-trace", "turn-trace", "trace-xyz");
        core.emit_agent(crate::agent::events::AgentDomainEvent::TextDelta {
            turn_id: "conv-trace".to_string(),
            delta: "hello".to_string(),
            trace_id: None,
        });
        drop(guard);
        core.emit_agent(crate::agent::events::AgentDomainEvent::TextDelta {
            turn_id: "conv-trace".to_string(),
            delta: "after".to_string(),
            trace_id: None,
        });

        let ev1 = rx.try_recv().expect("first event");
        match ev1 {
            NovaEvent::Agent(ev) => assert_eq!(ev.trace_id(), Some("trace-xyz")),
            _ => panic!("expected agent event"),
        }
        let ev2 = rx.try_recv().expect("second event");
        match ev2 {
            NovaEvent::Agent(ev) => assert_eq!(ev.trace_id(), None),
            _ => panic!("expected agent event"),
        }
    }
}
