use crate::core::Core;
use crate::events::NovaEvent;
use nova_protocol::mcp::McpElicitationDecision;
use rmcp::model::{ElicitRequestParams, ElicitResult, ElicitationAction, ErrorData as McpError};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const MCP_ELICITATION_TIMEOUT: Duration = Duration::from_secs(300);
const ELICITATION_OWNER_TTL: Duration = Duration::from_secs(3600);
const MAX_TRACKED_ELICITATION_OWNERS: usize = 512;

tokio::task_local! {
    static ACTIVE_CONVERSATION_ID: Option<String>;
}

struct PendingElicitation {
    client_id: Option<String>,
    sender: oneshot::Sender<McpElicitationDecision>,
}

#[derive(Clone)]
struct ServerInvocationContext {
    scope_id: u64,
    conversation_id: Option<String>,
    client_id: Option<String>,
}

#[derive(Default)]
struct ElicitationRegistry {
    pending: HashMap<String, PendingElicitation>,
    owners: HashMap<String, (Option<String>, Instant)>,
    owner_order: VecDeque<String>,
}

impl ElicitationRegistry {
    fn record_owner(&mut self, request_id: &str, client_id: Option<String>) {
        let now = Instant::now();
        while self.owners.len() >= MAX_TRACKED_ELICITATION_OWNERS {
            let Some(oldest) = self.owner_order.pop_front() else {
                break;
            };
            self.owners.remove(&oldest);
        }
        self.owners.retain(|_, (_, ts)| now.saturating_duration_since(*ts) <= ELICITATION_OWNER_TTL);
        self.owners.insert(request_id.to_string(), (client_id, now));
        self.owner_order.push_back(request_id.to_string());
    }
}

static PENDING_ELICITATIONS: OnceLock<Mutex<ElicitationRegistry>> = OnceLock::new();
static ACTIVE_SERVER_SCOPES: OnceLock<Mutex<HashMap<String, Vec<ServerInvocationContext>>>> =
    OnceLock::new();
static NEXT_SCOPE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn pending_map() -> &'static Mutex<ElicitationRegistry> {
    PENDING_ELICITATIONS.get_or_init(|| Mutex::new(ElicitationRegistry::default()))
}

struct PendingElicitationGuard {
    request_id: String,
}

impl Drop for PendingElicitationGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = pending_map().lock() {
            guard.pending.remove(&self.request_id);
        }
    }
}

fn server_scopes_map() -> &'static Mutex<HashMap<String, Vec<ServerInvocationContext>>> {
    ACTIVE_SERVER_SCOPES.get_or_init(|| Mutex::new(HashMap::new()))
}

struct ServerScopeGuard {
    server_name: String,
    scope_id: u64,
}

impl Drop for ServerScopeGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = server_scopes_map().lock() {
            if let Some(entries) = guard.get_mut(&self.server_name) {
                entries.retain(|entry| entry.scope_id != self.scope_id);
                if entries.is_empty() {
                    guard.remove(&self.server_name);
                }
            }
        }
    }
}

/// 在当前异步调用生命周期内绑定 `(server_name, conversation_id, client_id)`，
/// 使 `rmcp` 后台服务任务收到的 `elicitation/create` 能够精确归属并路由至发起客户端窗口。
pub(crate) async fn scope_conversation<F, T>(
    server_name: &str,
    conversation_id: Option<String>,
    fut: F,
) -> T
where
    F: Future<Output = T>,
{
    let scope_id = NEXT_SCOPE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let client_id = crate::agent::session_runtime::active_client_id(conversation_id.as_deref());
    if let Ok(mut guard) = server_scopes_map().lock() {
        guard
            .entry(server_name.to_string())
            .or_default()
            .push(ServerInvocationContext {
                scope_id,
                conversation_id: conversation_id.clone(),
                client_id,
            });
    }
    let _guard = ServerScopeGuard {
        server_name: server_name.to_string(),
        scope_id,
    };
    ACTIVE_CONVERSATION_ID.scope(conversation_id, fut).await
}

fn resolve_invocation_context(
    server_name: &str,
) -> Result<(Option<String>, Option<String>), McpError> {
    if let Some(conv_id) = ACTIVE_CONVERSATION_ID
        .try_with(|id| id.clone())
        .ok()
        .flatten()
    {
        let client_id = crate::agent::session_runtime::active_client_id(Some(&conv_id));
        return Ok((Some(conv_id), client_id));
    }

    let Ok(guard) = server_scopes_map().lock() else {
        return Ok((None, None));
    };
    let Some(entries) = guard.get(server_name) else {
        return Ok((None, None));
    };
    let Some(last) = entries.last() else {
        return Ok((None, None));
    };
    // 仅当全部活跃调用归属同一 client_id（含单并发场景）时直接绑定，拒绝跨窗口模糊归属。
    let single_owner = entries
        .iter()
        .all(|entry| entry.client_id == last.client_id);
    if single_owner {
        Ok((last.conversation_id.clone(), last.client_id.clone()))
    } else {
        Err(McpError::invalid_request(
            "Ambiguous concurrent MCP tool calls across multiple client windows; cannot route elicitation safely",
            None,
        ))
    }
}

/// 查询某条待处理 Elicitation 请求归属的发起连接 ID（供多窗口按连接隔离事件推送）。
pub fn pending_elicitation_client_id(request_id: &str) -> Option<String> {
    let guard = pending_map().lock().ok()?;
    guard
        .pending
        .get(request_id)
        .and_then(|entry| entry.client_id.clone())
        .or_else(|| {
            guard
                .owners
                .get(request_id)
                .and_then(|(cid, _)| cid.clone())
        })
}

/// 客户端连接断开时 fail-closed：把该连接持有的全部待处理 Elicitation 立即取消。
pub fn cancel_elicitations_for_client(client_id: &str) -> usize {
    let mut senders = Vec::new();
    {
        let Ok(mut guard) = pending_map().lock() else {
            return 0;
        };
        let keys: Vec<String> = guard
            .pending
            .iter()
            .filter_map(|(req_id, entry)| {
                if entry.client_id.as_deref() == Some(client_id) {
                    Some(req_id.clone())
                } else {
                    None
                }
            })
            .collect();
        for key in keys {
            if let Some(entry) = guard.pending.remove(&key) {
                senders.push(entry.sender);
            }
        }
    }
    let count = senders.len();
    for sender in senders {
        let _ = sender.send(McpElicitationDecision {
            action: "cancel".to_string(),
            content: None,
        });
    }
    count
}

/// 由前端或测试调用：回填指定 `request_id` 的 Elicitation 用户交互决策。
pub fn submit_elicitation_decision(
    request_id: &str,
    decision: McpElicitationDecision,
    caller_client_id: Option<&str>,
) -> Result<bool, String> {
    let sender = {
        let mut guard = pending_map()
            .lock()
            .map_err(|_| "MCP elicitation state unavailable".to_string())?;
        if let Some(entry) = guard.pending.get(request_id) {
            if let Some(owner) = entry.client_id.as_deref() {
                if caller_client_id != Some(owner) {
                    return Err("该交互请求归属另一个已连接窗口，当前连接无权处理。".to_string());
                }
            }
        }
        guard.pending.remove(request_id).map(|entry| entry.sender)
    };
    let Some(sender) = sender else {
        return Ok(false);
    };
    let _ = sender.send(decision);
    Ok(true)
}

fn validate_elicitation_url(raw_url: &str) -> Result<(), McpError> {
    let parsed = reqwest::Url::parse(raw_url)
        .map_err(|e| McpError::invalid_params(format!("Invalid elicitation URL: {e}"), None))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(McpError::invalid_params(
            format!(
                "Unsupported elicitation URL scheme '{}'; only http/https allowed",
                parsed.scheme()
            ),
            None,
        ));
    }
    Ok(())
}

fn parse_action(raw: &str) -> ElicitationAction {
    match raw.trim().to_ascii_lowercase().as_str() {
        "accept" => ElicitationAction::Accept,
        "decline" => ElicitationAction::Decline,
        _ => ElicitationAction::Cancel,
    }
}

/// 处理来自 MCP 服务端的 `elicitation/create` 请求，通过事件广播等待前端响应；
/// 发起客户端断连或超时一律 fail-closed 返回 `Cancel`。
pub(super) async fn request_elicitation(
    core: &Core,
    server_name: &str,
    params: ElicitRequestParams,
    ct: CancellationToken,
) -> Result<ElicitResult, McpError> {
    let (mode, message, url, requested_schema) = match params {
        ElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } => {
            let schema_val = serde_json::to_value(&requested_schema).map_err(|e| {
                McpError::invalid_params(format!("Invalid elicitation schema: {e}"), None)
            })?;
            ("form".to_string(), message, None, Some(schema_val))
        }
        ElicitRequestParams::UrlElicitationParams { message, url, .. } => {
            validate_elicitation_url(&url)?;
            ("url".to_string(), message, Some(url), None)
        }
        #[allow(unreachable_patterns)]
        _ => {
            return Ok(ElicitResult::new(ElicitationAction::Decline));
        }
    };

    let (conversation_id, owner_client_id) = resolve_invocation_context(server_name)?;
    if let Some(owner) = owner_client_id.as_deref() {
        if crate::agent::session_runtime::is_client_disconnected(owner) {
            return Ok(ElicitResult::new(ElicitationAction::Cancel));
        }
    }

    let request_id = format!("mcp-elicit-{}", uuid::Uuid::new_v4());
    let (tx, rx) = oneshot::channel::<McpElicitationDecision>();

    {
        let mut guard = pending_map()
            .lock()
            .map_err(|_| McpError::internal_error("Elicitation state poisoned", None))?;
        guard.record_owner(&request_id, owner_client_id.clone());
        guard.pending.insert(
            request_id.clone(),
            PendingElicitation {
                client_id: owner_client_id,
                sender: tx,
            },
        );
    }
    let _pending_guard = PendingElicitationGuard {
        request_id: request_id.clone(),
    };

    core.emit(NovaEvent::McpElicitationRequested {
        request_id: request_id.clone(),
        server_name: server_name.to_string(),
        mode,
        message,
        url,
        requested_schema,
        conversation_id,
    });

    let outcome = tokio::select! {
        biased;
        _ = ct.cancelled() => {
            return Ok(ElicitResult::new(ElicitationAction::Cancel));
        }
        res = tokio::time::timeout(MCP_ELICITATION_TIMEOUT, rx) => res,
    };

    match outcome {
        Ok(Ok(decision)) => {
            let action = parse_action(&decision.action);
            let is_accept = matches!(action, ElicitationAction::Accept);
            let mut result = ElicitResult::new(action);
            if is_accept {
                if let Some(content) = decision.content.filter(|v| !matches!(v, Value::Null)) {
                    result = result.with_content(content);
                }
            }
            Ok(result)
        }
        Ok(Err(_)) | Err(_) => Ok(ElicitResult::new(ElicitationAction::Cancel)),
    }
}
