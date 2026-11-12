//! 宿主服务端：内核经 **stdio JSONL**（CLI/单会话）、**WebSocket over UDS**（多窗口本地共享守护进程）
//! 或 **WebSocket over TCP**（远程可选）对外开放。
//!
//! 所有传输共用 `ClientHub`、`handle_message`、`event_pump` 与 `dispatch`，按连接身份 `ConnectionId`
//! 隔离待审批请求与桌面能力回调，确保任一客户端断开时由其持有的待审批立即 fail-closed 为 `DenyOnce`。

mod dispatch;

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use nova_client::uds::UnixListener as PrivateUnixListener;
use nova_core::{BrowserCapability, Core, HostHooks, NovaEvent};
use nova_protocol::agent::AgentDomainEvent;
use nova_protocol::sync::MutexExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tracing::{info, warn};

use crate::host;

/// 单连接并发请求上限；超限返回 JSON-RPC -32001。
const MAX_INFLIGHT_REQUESTS: usize = 32;

/// 出站队列容量。写满说明客户端消费不过来，丢弃并告警，而不是无限占用内存。
const OUTBOUND_QUEUE_CAPACITY: usize = 1024;

// ─────────────────────────────────────────────
// 启动引导（所有传输共用）
// ─────────────────────────────────────────────

/// 构建内核、取得单实例锁、挂载宿主能力、拉起后台子系统。
///
/// 返回的 `File` 是实例锁句柄，调用方必须持有到进程结束——一旦 drop，锁即释放。
async fn bootstrap() -> anyhow::Result<(Core, ClientHub, File)> {
    let core = host::build_core()?;
    let instance_lock = acquire_instance_lock(&core).map_err(anyhow::Error::msg)?;

    let hub = ClientHub::default();
    let core = core
        .with_browser(Arc::new(RemoteBrowser { hub: hub.clone() }))
        .with_hooks(Arc::new(RemoteHooks { hub: hub.clone() }));

    host::init_kernel(&core).await;

    // 后台子系统：MCP 预热 + 定时任务调度（与桌面端同源）。
    let warmup = core.clone();
    tokio::spawn(async move {
        if let Err(error) = nova_core::api::mcp::warmup_runtime(&warmup).await {
            warn!(%error, "mcp runtime warmup failed");
        }
    });
    let scheduler = core.clone();
    tokio::spawn(async move { nova_core::api::cron::run_scheduler_loop(&scheduler).await });

    Ok((core, hub, instance_lock))
}

/// 单实例锁：同一数据目录下只允许一个内核持有后台子系统。
fn acquire_instance_lock(core: &Core) -> Result<File, String> {
    use fs2::FileExt;
    use std::io::Write;

    let path = core.config().data_dir().join("kernel.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("无法打开内核锁文件 {}: {}", path.display(), e))?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.set_len(0);
            let mut handle = &file;
            let _ = writeln!(handle, "{}", std::process::id());
            info!(lock = %path.display(), pid = std::process::id(), "内核实例锁已获取");
            Ok(file)
        }
        Err(_) => {
            let holder = std::fs::read_to_string(&path).unwrap_or_default();
            let holder = holder.trim();
            let holder = if holder.is_empty() { "未知" } else { holder };
            Err(format!(
                "已有 Nova 内核实例运行中（pid {}）。请先退出该实例，或确认没有残留的 nova serve 进程。",
                holder
            ))
        }
    }
}

struct ServerState {
    core: Core,
    hub: ClientHub,
    inflight: Arc<Semaphore>,
    shutdown_tx: watch::Sender<bool>,
}

// ─────────────────────────────────────────────
// 按连接身份（ConnectionId）隔离的客户端枢纽
// ─────────────────────────────────────────────

#[derive(Clone)]
struct ClientConnection {
    out_tx: mpsc::Sender<Value>,
    has_browser: bool,
    has_hooks: bool,
}

struct PendingCapability {
    target_client_id: Option<String>,
    tx: oneshot::Sender<Value>,
}

#[derive(Clone, Default)]
struct ClientHub {
    inner: Arc<Mutex<HubInner>>,
}

#[derive(Default)]
struct HubInner {
    connections: HashMap<String, ClientConnection>,
    desktop_order: Vec<String>,
    pending: HashMap<String, PendingCapability>,
    counter: u64,
    conn_counter: u64,
}

impl ClientHub {
    fn register_connection(
        &self,
        out_tx: mpsc::Sender<Value>,
        has_browser: bool,
        has_hooks: bool,
    ) -> String {
        let mut inner = self.inner.lock_unpoisoned();
        inner.conn_counter += 1;
        let client_id = format!("conn-{}", inner.conn_counter);
        inner.connections.insert(
            client_id.clone(),
            ClientConnection {
                out_tx,
                has_browser,
                has_hooks,
            },
        );
        if has_browser {
            inner.desktop_order.push(client_id.clone());
        }
        client_id
    }

    fn update_capabilities(&self, client_id: &str, has_browser: bool, has_hooks: bool) {
        let mut inner = self.inner.lock_unpoisoned();
        if let Some(conn) = inner.connections.get_mut(client_id) {
            conn.has_browser = has_browser;
            conn.has_hooks = has_hooks;
        }
        inner.desktop_order.retain(|id| id != client_id);
        if has_browser {
            inner.desktop_order.push(client_id.to_string());
        }
    }

    /// 连接退出或断开时清理注册，立即拒绝挂起的浏览器能力请求并 fail-closed 所有待审批。
    fn unregister_connection(&self, core: &Core, client_id: &str) {
        let mut orphaned_senders = Vec::new();
        {
            let mut inner = self.inner.lock_unpoisoned();
            inner.connections.remove(client_id);
            inner.desktop_order.retain(|id| id != client_id);

            let matching_ids: Vec<String> = inner
                .pending
                .iter()
                .filter(|(_, item)| item.target_client_id.as_deref() == Some(client_id))
                .map(|(req_id, _)| req_id.clone())
                .collect();
            for req_id in matching_ids {
                if let Some(pending) = inner.pending.remove(&req_id) {
                    orphaned_senders.push(pending.tx);
                }
            }
        }

        for tx in orphaned_senders {
            let _ = tx.send(json!({
                "ok": false,
                "error": "originating desktop client disconnected",
            }));
        }

        let denied = nova_core::agent::permissions::deny_pending_for_client(core, client_id);
        if denied > 0 {
            info!(
                client_id = %client_id,
                denied_count = denied,
                "client disconnected; fail-closed pending permissions as DenyOnce"
            );
        }
    }

    fn next_request(
        &self,
        target_client_id: Option<String>,
    ) -> (String, oneshot::Receiver<Value>) {
        let mut inner = self.inner.lock_unpoisoned();
        inner.counter += 1;
        let id = format!("cap-{}", inner.counter);
        let (tx, rx) = oneshot::channel();
        inner.pending.insert(
            id.clone(),
            PendingCapability {
                target_client_id,
                tx,
            },
        );
        (id, rx)
    }

    fn resolve(&self, id: &str, payload: Value) {
        if let Some(pending) = self.inner.lock_unpoisoned().pending.remove(id) {
            let _ = pending.tx.send(payload);
        }
    }

    fn send_browser_to_client(
        &self,
        target_client_id: Option<&str>,
        message: Value,
    ) -> Result<(), String> {
        let inner = self.inner.lock_unpoisoned();
        let conn = match target_client_id {
            Some(client_id) => inner
                .connections
                .get(client_id)
                .filter(|c| c.has_browser)
                .ok_or_else(|| "originating desktop client disconnected or lacks browser capability".to_string())?,
            None => inner
                .desktop_order
                .iter()
                .rev()
                .find_map(|id| inner.connections.get(id).filter(|c| c.has_browser))
                .ok_or_else(|| "no desktop client connected".to_string())?,
        };
        conn.out_tx
            .try_send(message)
            .map_err(|_| "desktop client channel closed or full".to_string())
    }

    fn broadcast_hooks(&self, message: Value) {
        let inner = self.inner.lock_unpoisoned();
        for conn in inner.connections.values() {
            if conn.has_hooks || conn.has_browser {
                let _ = conn.out_tx.try_send(message.clone());
            }
        }
    }
}

struct RemoteBrowser {
    hub: ClientHub,
}

#[async_trait]
impl BrowserCapability for RemoteBrowser {
    async fn run_command(
        &self,
        conversation_id: Option<&str>,
        action: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Value {
        let target_client_id = nova_core::agent::session_runtime::active_client_id(conversation_id);
        let (id, rx) = self.hub.next_request(target_client_id.clone());
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "capability/browser",
            "params": {
                "conversationId": conversation_id,
                "action": action,
                "payload": payload,
                "timeoutMs": timeout_ms,
            },
        });
        if let Err(error) = self
            .hub
            .send_browser_to_client(target_client_id.as_deref(), request)
        {
            self.hub.resolve(&id, json!({ "ok": false, "error": error }));
            return json!({ "ok": false, "error": error });
        }
        match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms.unwrap_or(120_000)),
            rx,
        )
        .await
        {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => json!({ "ok": false, "error": "desktop capability channel closed" }),
            Err(_) => json!({ "ok": false, "error": "desktop capability request timed out" }),
        }
    }
}

struct RemoteHooks {
    hub: ClientHub,
}

impl RemoteHooks {
    fn notify(&self, operation: &str, params: Value) {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "capability/hooks",
            "params": { "operation": operation, "params": params },
        });
        self.hub.broadcast_hooks(message);
    }
}

impl HostHooks for RemoteHooks {
    fn stop_user_terminal_session(&self, conversation_id: &str) {
        self.notify(
            "stop_user_terminal_session",
            json!({ "conversationId": conversation_id }),
        );
    }

    fn close_all_user_terminal_sessions(&self) {
        self.notify("close_all_user_terminal_sessions", json!({}));
    }

    fn set_file_logging_enabled(&self, enabled: bool) {
        self.notify("set_file_logging_enabled", json!({ "enabled": enabled }));
    }
}

// ─────────────────────────────────────────────
// 共用消息处理与按连接过滤事件泵
// ─────────────────────────────────────────────

/// 判断事件是否绑定了特定发起连接：仅限发起审批的窗口接收 `PermissionRequested`。
fn should_deliver_event_to_client(event: &NovaEvent, client_id: &str) -> bool {
    if let NovaEvent::Agent(AgentDomainEvent::PermissionRequested {
        turn_id,
        request_id,
        ..
    }) = event
    {
        if let Some(owner_client_id) =
            nova_core::agent::permissions::pending_approval_client_id(Some(turn_id), request_id)
        {
            return owner_client_id == client_id;
        }
    }
    true
}

/// 内核事件扇出：把领域事件转成稳定的前端通知，按连接隔离审批事件后写入出站通道。
async fn event_pump(core: Core, client_id: String, out_tx: mpsc::Sender<Value>) {
    let mut events = core.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => {
                if !should_deliver_event_to_client(&event, &client_id) {
                    continue;
                }
                let (name, payload) = event.to_frontend();
                let notification = json!({
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": { "name": name, "payload": payload },
                });
                match out_tx.try_send(notification) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!("出站队列已满，丢弃一条内核事件");
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// 单条入站信封的完整处理：能力响应回填 / 握手注册 / 业务分发。
async fn handle_message(
    state: &Arc<ServerState>,
    client_id: &str,
    envelope: Value,
    out_tx: &mpsc::Sender<Value>,
) {
    // 能力请求响应（无 method，携带 id + result/error）。
    if envelope.get("method").is_none() {
        if let Some(id) = envelope.get("id").and_then(Value::as_str) {
            let payload = envelope
                .get("result")
                .cloned()
                .unwrap_or_else(|| envelope.get("error").cloned().unwrap_or(Value::Null));
            state.hub.resolve(id, payload);
        }
        return;
    }

    let method = envelope.get("method").and_then(Value::as_str).unwrap_or("");
    let id = envelope.get("id").cloned();
    let params = envelope.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let client = params.get("clientInfo");
            let client_name = client
                .and_then(|c| c.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let client_version = client
                .and_then(|c| c.get("version"))
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            info!(
                client_id = %client_id,
                client = client_name,
                version = client_version,
                capabilities = ?params.get("capabilities"),
                "initialize"
            );

            let client_protocol = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !client_protocol.is_empty()
                && client_protocol != nova_protocol::rpc::PROTOCOL_VERSION
            {
                let _ = out_tx.try_send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32600,
                        "message": format!(
                            "protocol version mismatch: client={}, server={}",
                            client_protocol,
                            nova_protocol::rpc::PROTOCOL_VERSION
                        ),
                    },
                }));
                return;
            }

            let wants_browser = params
                .pointer("/capabilities/browser")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let wants_hooks = params
                .pointer("/capabilities/hooks")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state
                .hub
                .update_capabilities(client_id, wants_browser, wants_hooks);

            let result = serde_json::to_value(nova_protocol::rpc::InitializeResult {
                server_info: nova_protocol::rpc::ClientInfo {
                    name: "nova".to_string(),
                    title: Some("Nova Agent Kernel".to_string()),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
                protocol_version: nova_protocol::rpc::PROTOCOL_VERSION.to_string(),
                capabilities: nova_protocol::rpc::ServerCapabilities {
                    browser: true,
                    hooks: true,
                    escalation: true,
                },
            })
            .unwrap_or(Value::Null);

            let _ = out_tx.try_send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }));
        }
        "initialized" => {
            info!(client_id = %client_id, "client initialized");
        }
        "hello" => {
            let is_desktop = params
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|caps| caps.iter().any(|c| c.as_str() == Some("browser")))
                .unwrap_or(false);
            if is_desktop {
                state.hub.update_capabilities(client_id, true, true);
            }
            let _ = out_tx.try_send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": true },
            }));
        }
        "shutdown" => {
            let _ = out_tx
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": true },
                }))
                .await;
            let _ = state.shutdown_tx.send(true);
        }
        _ => {
            let permit = match state.inflight.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    let _ = out_tx.try_send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32001, "message": "Server overloaded; retry later." },
                    }));
                    return;
                }
            };

            let state = Arc::clone(state);
            let out_tx = out_tx.clone();
            let method = method.to_string();
            let client_id = client_id.to_string();
            tokio::spawn(async move {
                let _permit = permit;
                let result =
                    dispatch::dispatch(&state.core, method.as_str(), params, Some(&client_id)).await;
                let response = match result {
                    Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
                    Err((code, message)) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": code, "message": message },
                    }),
                };
                let _ = out_tx.try_send(response);
            });
        }
    }
}

// ─────────────────────────────────────────────
// stdio 传输（单次会话 / 无头 / TUI）
// ─────────────────────────────────────────────

pub async fn run_stdio() -> anyhow::Result<()> {
    let (core, hub, _instance_lock) = bootstrap().await?;
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let state = Arc::new(ServerState {
        core,
        hub,
        inflight: Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS)),
        shutdown_tx,
    });

    let (out_tx, mut out_rx) = mpsc::channel::<Value>(OUTBOUND_QUEUE_CAPACITY);
    let client_id = state.hub.register_connection(out_tx.clone(), true, true);

    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = out_rx.recv().await {
            let Ok(mut line) = serde_json::to_vec(&message) else {
                continue;
            };
            line.push(b'\n');
            if stdout.write_all(&line).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });

    let pump = tokio::spawn(event_pump(
        state.core.clone(),
        client_id.clone(),
        out_tx.clone(),
    ));

    info!("nova daemon serving over stdio");

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            next = lines.next_line() => match next {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(envelope) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    handle_message(&state, &client_id, envelope, &out_tx).await;
                }
                Ok(None) => break,
                Err(error) => {
                    warn!(%error, "stdio 读取失败，守护进程退出");
                    break;
                }
            }
        }
    }

    nova_core::services::unified_exec::close_all().await;
    state.core.hooks().close_all_user_terminal_sessions();

    pump.abort();
    state.hub.unregister_connection(&state.core, &client_id);
    drop(out_tx);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
    info!("nova daemon stdio channel closed, exiting");
    Ok(())
}

// ─────────────────────────────────────────────
// WebSocket over UDS（多窗口本地共享守护进程）
// ─────────────────────────────────────────────

pub async fn run_uds(socket_path: &Path) -> anyhow::Result<()> {
    let (core, hub, _instance_lock) = bootstrap().await?;
    let mut listener = PrivateUnixListener::bind(socket_path)
        .await
        .map_err(|e| anyhow::anyhow!("绑定私有 UDS 失败 ({}): {e}", socket_path.display()))?;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let state = Arc::new(ServerState {
        core: core.clone(),
        hub,
        inflight: Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS)),
        shutdown_tx,
    });

    info!(socket_path = %socket_path.display(), "Nova daemon listening (websocket over UDS)");

    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok(stream) => {
                        if let Err(error) = stream.ensure_trusted_peer() {
                            warn!(%error, "rejected untrusted UDS peer");
                            continue;
                        }
                        let state = Arc::clone(&state);
                        tokio::spawn(async move {
                            match tokio_tungstenite::accept_async(stream).await {
                                Ok(ws) => handle_tungstenite_socket(ws, state).await,
                                Err(error) => warn!(%error, "UDS websocket upgrade failed"),
                            }
                        });
                    }
                    Err(error) => {
                        warn!(%error, "UDS accept failed");
                    }
                }
            }
        }
    }

    nova_core::services::unified_exec::close_all().await;
    core.hooks().close_all_user_terminal_sessions();
    let _ = tokio::fs::remove_file(socket_path).await;
    info!("nova UDS daemon shut down cleanly");
    Ok(())
}

async fn drive_ws_connection<M, E, Si, St>(
    state: Arc<ServerState>,
    mut sink: Si,
    mut stream: St,
    wrap_text: fn(String) -> M,
    extract_text: fn(M) -> Option<String>,
) where
    M: Send + 'static,
    E: Send + 'static,
    Si: futures_util::Sink<M> + Unpin + Send + 'static,
    St: futures_util::Stream<Item = Result<M, E>> + Unpin + Send + 'static,
{
    let (out_tx, mut out_rx) = mpsc::channel::<Value>(OUTBOUND_QUEUE_CAPACITY);
    let client_id = state.hub.register_connection(out_tx.clone(), false, false);

    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if sink.send(wrap_text(text)).await.is_err() {
                break;
            }
        }
    });

    let pump = tokio::spawn(event_pump(
        state.core.clone(),
        client_id.clone(),
        out_tx.clone(),
    ));

    let reader_state = Arc::clone(&state);
    let reader_client_id = client_id.clone();
    let reader_out = out_tx.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            let Some(text) = extract_text(message) else {
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            handle_message(&reader_state, &reader_client_id, envelope, &reader_out).await;
        }
    });

    tokio::select! {
        _ = writer => {},
        _ = pump => {},
        _ = reader => {},
    }

    state.hub.unregister_connection(&state.core, &client_id);
}

async fn handle_tungstenite_socket(
    ws: tokio_tungstenite::WebSocketStream<nova_client::uds::UnixStream>,
    state: Arc<ServerState>,
) {
    let (sink, stream) = ws.split();
    drive_ws_connection(
        state,
        sink,
        stream,
        |text| TungsteniteMessage::Text(text.into()),
        |msg| match msg {
            TungsteniteMessage::Text(text) => Some(text.to_string()),
            _ => None,
        },
    )
    .await;
}

// ─────────────────────────────────────────────
// WebSocket over TCP（远程 / 可选）
// ─────────────────────────────────────────────

pub async fn run(host_addr: &str, port: u16) -> anyhow::Result<()> {
    let (core, hub, _instance_lock) = bootstrap().await?;
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let state = Arc::new(ServerState {
        core: core.clone(),
        hub,
        inflight: Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS)),
        shutdown_tx,
    });

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/healthz", get(|| async { "OK" }))
        .route("/readyz", get(|| async { "OK" }))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((host_addr, port)).await?;
    let addr = listener.local_addr()?;
    info!("Nova daemon listening on {} (websocket)", addr);
    eprintln!("Nova Daemon 已就绪（WebSocket 远程模式）: ws://{}/ws", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown_rx.borrow() {
                if shutdown_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await?;
    nova_core::services::unified_exec::close_all().await;
    core.hooks().close_all_user_terminal_sessions();
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<ServerState>) {
    let (sink, stream) = socket.split();
    drive_ws_connection(
        state,
        sink,
        stream,
        |text| AxumMessage::Text(text.into()),
        |msg| match msg {
            AxumMessage::Text(text) => Some(text.to_string()),
            _ => None,
        },
    )
    .await;
}
