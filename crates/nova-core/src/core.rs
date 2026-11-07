//! 内核唯一上下文：配置 + 事件枢纽 + 宿主能力。
//!
//! 所有领域模块（引擎/工具/权限/会话/Provider）只依赖 `Core`，
//! 不感知 CLI、守护进程、桌面端等任何宿主形态。

use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::config::CoreConfig;
use crate::events::{EventHub, NovaEvent};

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
}

/// 内核上下文句柄（Arc 共享，廉价克隆，线程安全）。
#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
}

impl Core {
    pub fn new(config: CoreConfig, event_capacity: usize) -> Self {
        Self {
            inner: Arc::new(CoreInner {
                config,
                events: EventHub::new(event_capacity),
                browser: RwLock::new(None),
                hooks: RwLock::new(Arc::new(NoopHostHooks)),
            }),
        }
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

    /// Agent 领域事件快捷出口（最高频事件类型）。
    pub fn emit_agent(&self, event: crate::agent::events::AgentDomainEvent) {
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
