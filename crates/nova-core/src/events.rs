//! 内核统一事件系统：全类型化，零字符串事件名。
//!
//! 内核只产生 `NovaEvent`；宿主各自决定传输与呈现：
//! - CLI TUI：进程内订阅，直接匹配枚举渲染；
//! - 守护进程：`to_frontend()` 序列化为协议通知转发；
//! - 桌面端：`to_frontend()` 还原为前端既有事件契约（事件名/载荷 1:1）。

use tokio::sync::broadcast;

use crate::agent::events::AgentDomainEvent;
use crate::agent::utils::error_event::BackendErrorEvent;
use crate::provider::types::ChatMessageEvent;

/// 内核领域事件总和。
#[derive(Debug, Clone)]
pub enum NovaEvent {
    /// Agent 领域事件（回合生命周期/流式增量/工具调用/权限请求/用量）。
    Agent(AgentDomainEvent),
    /// 分支问答流式增量。
    BranchStream {
        parent_conversation_id: String,
        branch_id: String,
        event: ChatMessageEvent,
    },
    /// 分支生命周期状态。
    BranchStatus {
        parent_conversation_id: String,
        branch_id: String,
        phase: String,
        detail: Option<String>,
    },
    /// 子代理流式增量。
    SubagentStream {
        parent_conversation_id: String,
        sub_id: String,
        event: ChatMessageEvent,
    },
    /// 子代理生命周期状态。
    SubagentStatus {
        parent_conversation_id: String,
        sub_id: String,
        phase: String,
        task: String,
        detail: Option<String>,
        report_preview: Option<String>,
        elapsed_ms: Option<u64>,
    },
    /// 后端错误（用户可见）。
    BackendError(BackendErrorEvent),
    /// 后端警告（容忍但信息丢失，用户可见）。
    BackendWarning(BackendErrorEvent),
    /// 待办列表变更。
    TodoUpdated { conversation_id: Option<String> },
    /// 计划面板变更。
    PlanUpdated {
        conversation_id: Option<String>,
        content: String,
        updated_at: i64,
    },
    /// 定时任务触发（载荷结构由 cron 域定义）。
    ScheduledTaskTrigger(serde_json::Value),
    /// 智能体图谱节点变更。
    AgentGraphUpdated {
        conversation_id: String,
        node: serde_json::Value,
    },
    /// 协同看板新帖子。
    BoardPostCreated {
        conversation_id: String,
        post: serde_json::Value,
    },
    /// 协同看板频道变更。
    BoardChannelCreated {
        conversation_id: String,
        channel: serde_json::Value,
    },
}

impl NovaEvent {
    /// 前端线协议映射（唯一事实源）：事件名 + JSON 载荷。
    /// 与历史桌面端契约完全一致，前端无需任何改动。
    pub fn to_frontend(&self) -> (&'static str, serde_json::Value) {
        match self {
            NovaEvent::Agent(event) => ("agent-event", json(event)),
            NovaEvent::BranchStream {
                parent_conversation_id,
                branch_id,
                event,
            } => (
                "branch-event",
                serde_json::json!({
                    "kind": "stream",
                    "parentConversationId": parent_conversation_id,
                    "branchId": branch_id,
                    "event": event,
                }),
            ),
            NovaEvent::BranchStatus {
                parent_conversation_id,
                branch_id,
                phase,
                detail,
            } => (
                "branch-event",
                serde_json::json!({
                    "kind": "status",
                    "parentConversationId": parent_conversation_id,
                    "branchId": branch_id,
                    "phase": phase,
                    "detail": detail,
                }),
            ),
            NovaEvent::SubagentStream {
                parent_conversation_id,
                sub_id,
                event,
            } => (
                "subagent-event",
                serde_json::json!({
                    "kind": "stream",
                    "parentConversationId": parent_conversation_id,
                    "subId": sub_id,
                    "event": event,
                }),
            ),
            NovaEvent::SubagentStatus {
                parent_conversation_id,
                sub_id,
                phase,
                task,
                detail,
                report_preview,
                elapsed_ms,
            } => {
                let mut payload = serde_json::json!({
                    "kind": "status",
                    "parentConversationId": parent_conversation_id,
                    "subId": sub_id,
                    "phase": phase,
                    "task": task,
                    "detail": detail,
                });
                if let Some(preview) = report_preview {
                    payload["reportPreview"] = serde_json::json!(preview);
                }
                if let Some(ms) = elapsed_ms {
                    payload["elapsedMs"] = serde_json::json!(ms);
                }
                ("subagent-event", payload)
            }
            NovaEvent::BackendError(payload) => ("backend-error", json(payload)),
            NovaEvent::BackendWarning(payload) => ("backend-warning", json(payload)),
            NovaEvent::TodoUpdated { conversation_id } => (
                "todo-updated",
                serde_json::json!({ "conversationId": conversation_id }),
            ),
            NovaEvent::PlanUpdated {
                conversation_id,
                content,
                updated_at,
            } => (
                "plan-updated",
                serde_json::json!({
                    "conversationId": conversation_id,
                    "content": content,
                    "updatedAt": updated_at,
                }),
            ),
            NovaEvent::ScheduledTaskTrigger(payload) => {
                ("scheduled-task-trigger", payload.clone())
            }
            NovaEvent::AgentGraphUpdated { conversation_id, node } => (
                "agent-graph-updated",
                serde_json::json!({
                    "conversationId": conversation_id,
                    "node": node,
                }),
            ),
            NovaEvent::BoardPostCreated { conversation_id, post } => (
                "board-post-created",
                serde_json::json!({
                    "conversationId": conversation_id,
                    "post": post,
                }),
            ),
            NovaEvent::BoardChannelCreated { conversation_id, channel } => (
                "board-channel-created",
                serde_json::json!({
                    "conversationId": conversation_id,
                    "channel": channel,
                }),
            ),
        }
    }
}

fn json(value: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

/// 事件枢纽：Tokio broadcast 多订阅者分发。
#[derive(Clone)]
pub struct EventHub {
    sender: broadcast::Sender<NovaEvent>,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// 无订阅者时静默丢弃，不阻断业务流程。
    pub fn emit(&self, event: NovaEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NovaEvent> {
        self.sender.subscribe()
    }
}
