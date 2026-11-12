//! 智能体运行时注册表与异步生命周期状态机（对齐 MultiAgent V2 规范）。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock, RwLock};
use nova_protocol::sync::RwLockExt;
use std::time::Duration;
use tokio::sync::{watch, Mutex};
use tokio_util::sync::CancellationToken;
use serde::{Deserialize, Serialize};

/// 智能体运行态生命周期状态（对应 Codex AgentStatus 7 变体体系）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentLifecycleStatus {
    PendingInit,
    Running,
    Completed(Option<String>),
    Errored(String),
    Interrupted,
}

impl AgentLifecycleStatus {
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            AgentLifecycleStatus::Completed(_)
                | AgentLifecycleStatus::Errored(_)
                | AgentLifecycleStatus::Interrupted
        )
    }

    pub fn to_string_repr(&self) -> &'static str {
        match self {
            AgentLifecycleStatus::PendingInit => "pending_init",
            AgentLifecycleStatus::Running => "running",
            AgentLifecycleStatus::Completed(_) => "completed",
            AgentLifecycleStatus::Errored(_) => "errored",
            AgentLifecycleStatus::Interrupted => "interrupted",
        }
    }
}

/// 智能体异步执行句柄，维护后台任务、信箱、状态与取消令牌。
pub struct AgentHandle {
    pub id: String,
    pub parent_id: String,
    pub role: String,
    pub nickname: String,
    pub task: String,
    pub status: Arc<RwLock<AgentLifecycleStatus>>,
    pub cancel_token: CancellationToken,
    pub mailbox: Arc<Mutex<VecDeque<String>>>,
    pub mailbox_notify: watch::Sender<u64>,
    pub mailbox_rx: watch::Receiver<u64>,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub agent_id: String,
    pub role: String,
    pub nickname: String,
    pub task: String,
    pub status: String,
    pub is_final: bool,
    pub created_at: i64,
    pub result_preview: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WaitResult {
    pub status: String,
    pub target_agent_id: Option<String>,
    pub target_nickname: Option<String>,
    pub agent_status: String,
    pub report_preview: Option<String>,
    pub message: String,
}

type AgentBucket = HashMap<String, Arc<AgentHandle>>;

#[derive(Default)]
pub struct AgentRegistry {
    // 按 parent_conversation_id 组织 handles: parent_id -> sub_id -> AgentHandle
    agents: Arc<RwLock<HashMap<String, AgentBucket>>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册新的子智能体句柄。
    pub fn register(&self, handle: Arc<AgentHandle>) {
        let mut guard = self.agents.write_unpoisoned();
        let map = guard.entry(handle.parent_id.clone()).or_default();
        map.insert(handle.id.clone(), handle);
    }

    /// 原子检查并注册子智能体句柄：若同一父会话下仍处于非终态（PendingInit/Running）的数量
    /// 已达 `max_active_per_parent`，则拒绝注册并返回当前活跃数量，防止并发 TOCTOU 超限。
    pub fn try_register(&self, handle: Arc<AgentHandle>, max_active_per_parent: usize) -> Result<(), usize> {
        let mut guard = self.agents.write_unpoisoned();
        let map = guard.entry(handle.parent_id.clone()).or_default();
        let active = map
            .values()
            .filter(|h| !h.status.read_unpoisoned().is_final())
            .count();
        if active >= max_active_per_parent {
            return Err(active);
        }
        map.insert(handle.id.clone(), handle);
        Ok(())
    }

    /// 根据子智能体 ID 或 nickname 查询句柄。
    pub fn find(&self, parent_id: &str, target: &str) -> Option<Arc<AgentHandle>> {
        let guard = self.agents.read_unpoisoned();
        let map = guard.get(parent_id)?;
        if let Some(h) = map.get(target) {
            return Some(h.clone());
        }
        // 按 nickname 模糊匹配
        map.values().find(|h| h.nickname.eq_ignore_ascii_case(target)).cloned()
    }

    /// 会话删除时移除其全部子智能体句柄，并取消仍在运行的任务。
    pub fn remove_conversation(&self, parent_id: &str) {
        let mut guard = self.agents.write_unpoisoned();
        if let Some(map) = guard.remove(parent_id) {
            for handle in map.values() {
                handle.cancel_token.cancel();
            }
        }
    }

    /// 全量清理时移除所有会话的子智能体句柄并触发取消。
    pub fn remove_all(&self) {
        let mut guard = self.agents.write_unpoisoned();
        for (_, map) in guard.drain() {
            for handle in map.values() {
                handle.cancel_token.cancel();
            }
        }
    }

    /// 列出某会话下的所有子智能体摘要。
    pub fn list_summaries(&self, parent_id: &str) -> Vec<AgentSummary> {
        let guard = self.agents.read_unpoisoned();
        let Some(map) = guard.get(parent_id) else {
            return Vec::new();
        };

        let mut list: Vec<AgentSummary> = map
            .values()
            .map(|h| {
                let status_guard = h.status.read_unpoisoned();
                let status_repr = status_guard.to_string_repr().to_string();
                let is_final = status_guard.is_final();
                let result_preview = match &*status_guard {
                    AgentLifecycleStatus::Completed(Some(rep)) => {
                        Some(rep.chars().take(200).collect())
                    }
                    AgentLifecycleStatus::Errored(err) => Some(err.clone()),
                    _ => None,
                };
                AgentSummary {
                    agent_id: h.id.clone(),
                    role: h.role.clone(),
                    nickname: h.nickname.clone(),
                    task: h.task.clone(),
                    status: status_repr,
                    is_final,
                    created_at: h.created_at,
                    result_preview,
                }
            })
            .collect();

        list.sort_by_key(|s| s.created_at);
        list
    }

    /// 异步向指定子智能体的 Mailbox 投递消息。
    pub async fn send_to_mailbox(&self, parent_id: &str, target: &str, message: &str) -> Result<(), String> {
        let handle = self
            .find(parent_id, target)
            .ok_or_else(|| format!("Target agent '{}' not found", target))?;

        {
            let mut mb = handle.mailbox.lock().await;
            mb.push_back(message.to_string());
        }
        handle.mailbox_notify.send_modify(|val| *val = val.saturating_add(1));
        Ok(())
    }

    /// 中断指定的子智能体。
    pub fn interrupt(&self, parent_id: &str, target: &str) -> Result<(), String> {
        let handle = self
            .find(parent_id, target)
            .ok_or_else(|| format!("Target agent '{}' not found", target))?;

        handle.cancel_token.cancel();
        if let Ok(mut status) = handle.status.write() {
            *status = AgentLifecycleStatus::Interrupted;
        }
        handle.mailbox_notify.send_modify(|val| *val = val.saturating_add(1));
        Ok(())
    }

    /// 等待特定或任意活跃子智能体完成或状态更新（支持超时与取消）。
    pub async fn wait(
        &self,
        parent_id: &str,
        target: Option<&str>,
        timeout_duration: Duration,
        parent_cancel: CancellationToken,
    ) -> Result<WaitResult, String> {
        let timeout_sleep = tokio::time::sleep(timeout_duration);
        tokio::pin!(timeout_sleep);

        loop {
            // 检查目标是否已完结
            if let Some(tgt) = target {
                if let Some(handle) = self.find(parent_id, tgt) {
                    let status = handle.status.read_unpoisoned().clone();
                    if status.is_final() {
                        let (repr, rep) = match &status {
                            AgentLifecycleStatus::Completed(rep) => ("completed", rep.clone()),
                            AgentLifecycleStatus::Errored(err) => ("errored", Some(err.clone())),
                            AgentLifecycleStatus::Interrupted => ("interrupted", Some("Subagent was interrupted".into())),
                            _ => ("running", None),
                        };
                        return Ok(WaitResult {
                            status: "completed".into(),
                            target_agent_id: Some(handle.id.clone()),
                            target_nickname: Some(handle.nickname.clone()),
                            agent_status: repr.into(),
                            report_preview: rep,
                            message: format!("Agent '{}' reached final status: {}", handle.nickname, repr),
                        });
                    }
                } else {
                    return Err(format!("Target agent '{}' not found", tgt));
                }
            } else {
                // 等待任意子节点完成
                let summaries = self.list_summaries(parent_id);
                if summaries.is_empty() {
                    return Ok(WaitResult {
                        status: "no_agents".into(),
                        target_agent_id: None,
                        target_nickname: None,
                        agent_status: "none".into(),
                        report_preview: None,
                        message: "No subagents found in this conversation".into(),
                    });
                }
                if let Some(final_agent) = summaries.iter().find(|s| s.is_final) {
                    return Ok(WaitResult {
                        status: "completed".into(),
                        target_agent_id: Some(final_agent.agent_id.clone()),
                        target_nickname: Some(final_agent.nickname.clone()),
                        agent_status: final_agent.status.clone(),
                        report_preview: final_agent.result_preview.clone(),
                        message: format!("Agent '{}' completed with status {}", final_agent.nickname, final_agent.status),
                    });
                }
            }

            tokio::select! {
                _ = parent_cancel.cancelled() => {
                    return Ok(WaitResult {
                        status: "cancelled".into(),
                        target_agent_id: None,
                        target_nickname: None,
                        agent_status: "cancelled".into(),
                        report_preview: None,
                        message: "Wait cancelled by parent session".into(),
                    });
                }
                _ = &mut timeout_sleep => {
                    let summaries = self.list_summaries(parent_id);
                    let running_count = summaries.iter().filter(|s| !s.is_final).count();
                    return Ok(WaitResult {
                        status: "timed_out".into(),
                        target_agent_id: target.map(str::to_string),
                        target_nickname: None,
                        agent_status: "running".into(),
                        report_preview: None,
                        message: format!(
                            "Wait timed out after {}s ({} subagents still running).",
                            timeout_duration.as_secs(),
                            running_count
                        ),
                    });
                }
                _ = tokio::time::sleep(Duration::from_millis(300)) => {
                    continue;
                }
            }
        }
    }
}

pub fn global_agent_registry() -> &'static AgentRegistry {
    static REGISTRY: OnceLock<AgentRegistry> = OnceLock::new();
    REGISTRY.get_or_init(AgentRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;

    #[tokio::test]
    async fn test_registry_lifecycle_and_mailbox() {
        let registry = AgentRegistry::new();
        let (notify_tx, notify_rx) = watch::channel(0);

        let handle = Arc::new(AgentHandle {
            id: "agent_123".into(),
            parent_id: "conv_root".into(),
            role: "researcher".into(),
            nickname: "Explorer".into(),
            task: "Scan codebase".into(),
            status: Arc::new(RwLock::new(AgentLifecycleStatus::Running)),
            cancel_token: CancellationToken::new(),
            mailbox: Arc::new(Mutex::new(VecDeque::new())),
            mailbox_notify: notify_tx,
            mailbox_rx: notify_rx,
            created_at: 1000,
        });

        registry.register(handle.clone());

        // 1. Check find by id and nickname
        assert!(registry.find("conv_root", "agent_123").is_some());
        assert!(registry.find("conv_root", "explorer").is_some());

        // 2. Check list_summaries
        let list = registry.list_summaries("conv_root");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].status, "running");
        assert!(!list[0].is_final);

        // 3. Check send_to_mailbox
        registry.send_to_mailbox("conv_root", "Explorer", "Please check tests first").await.unwrap();
        {
            let mut mb = handle.mailbox.lock().await;
            assert_eq!(mb.pop_front().as_deref(), Some("Please check tests first"));
        }

        // 4. Check wait with timeout
        let wait_timeout = registry.wait("conv_root", Some("Explorer"), Duration::from_millis(50), CancellationToken::new()).await.unwrap();
        assert_eq!(wait_timeout.status, "timed_out");

        // 5. Interrupt
        registry.interrupt("conv_root", "Explorer").unwrap();
        assert!(handle.cancel_token.is_cancelled());
        assert_eq!(*handle.status.read().unwrap(), AgentLifecycleStatus::Interrupted);

        // 6. Wait completes immediately on final status
        let wait_done = registry.wait("conv_root", Some("Explorer"), Duration::from_millis(500), CancellationToken::new()).await.unwrap();
        assert_eq!(wait_done.status, "completed");
        assert_eq!(wait_done.agent_status, "interrupted");
    }
}

