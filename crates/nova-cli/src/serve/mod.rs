//! 宿主服务端：内核经 **stdio JSONL**（桌面默认）或 **WebSocket**（远程可选）对外开放。
//!
//! - 客户端请求：`{jsonrpc, id, method, params}` → 唯一分发器 → `{id, result|error}`；
//! - 内核事件：以 `{method: "event", params: {name, payload}}` 通知扇出（前端契约由内核 to_frontend 定义）；
//! - 桌面能力：浏览器自动化经 `capability/browser` 服务端→客户端请求闭环；
//!   宿主钩子经 `capability/hooks` 通知下发（火忘式）。
//!
//! 两种传输共用 `handle_message` / `event_pump`，差别只在字节通道：
//! - stdio：桌面壳 spawn 本进程并持有管道，不开端口，天然随父进程退出；
//! - WebSocket：远程 / 多客户端场景，需自行承担鉴权与单实例约束。

mod dispatch;

use std::collections::HashMap;
use std::fs::File;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use nova_core::{BrowserCapability, Core, HostHooks};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tracing::{info, warn};

use crate::host;

/// 单连接并发请求上限；超限返回 JSON-RPC -32001（与 Codex App Server 同语义）。
const MAX_INFLIGHT_REQUESTS: usize = 32;

/// 出站队列容量。写满说明客户端消费不过来，丢弃并告警，而不是无限占用内存。
const OUTBOUND_QUEUE_CAPACITY: usize = 1024;

// ─────────────────────────────────────────────
// 启动引导（两种传输共用）
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
///
/// 没有它，两个守护进程会各自跑一份 cron 调度，定时任务被执行两次。
/// TUI 与 `-p` 无头模式不持锁——它们只做单次会话，不争抢后台调度。
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
}

// ─────────────────────────────────────────────
// 客户端注册与能力请求路由
// ─────────────────────────────────────────────

#[derive(Clone, Default)]
struct ClientHub {
    inner: Arc<Mutex<HubInner>>,
}

#[derive(Default)]
struct HubInner {
    desktop: Option<mpsc::Sender<Value>>,
    pending: HashMap<String, oneshot::Sender<Value>>,
    counter: u64,
}

impl ClientHub {
    fn register_desktop(&self, sender: mpsc::Sender<Value>) {
        self.inner.lock().unwrap().desktop = Some(sender);
    }

    /// 摘掉桌面端出站句柄，用于连接收尾时让 writer 自然排空。
    fn clear_desktop(&self) {
        self.inner.lock().unwrap().desktop = None;
    }

    fn next_request(&self) -> (String, oneshot::Receiver<Value>) {
        let mut inner = self.inner.lock().unwrap();
        inner.counter += 1;
        let id = format!("cap-{}", inner.counter);
        let (tx, rx) = oneshot::channel();
        inner.pending.insert(id.clone(), tx);
        (id, rx)
    }

    fn resolve(&self, id: &str, payload: Value) {
        if let Some(tx) = self.inner.lock().unwrap().pending.remove(id) {
            let _ = tx.send(payload);
        }
    }

    fn send_to_desktop(&self, message: Value) -> Result<(), String> {
        let inner = self.inner.lock().unwrap();
        inner
            .desktop
            .as_ref()
            .ok_or_else(|| "no desktop client connected".to_string())?
            .try_send(message)
            .map_err(|_| "desktop client channel closed or full".to_string())
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
        let (id, rx) = self.hub.next_request();
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
        if let Err(error) = self.hub.send_to_desktop(request) {
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
        let _ = self.hub.send_to_desktop(message);
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
// 共用消息处理
// ─────────────────────────────────────────────

/// 内核事件扇出：把领域事件转成稳定的前端通知，写入给定通道。
async fn event_pump(core: Core, out_tx: mpsc::Sender<Value>) {
    let mut events = core.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => {
                let (name, payload) = event.to_frontend();
                let notification = json!({
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": { "name": name, "payload": payload },
                });
                match out_tx.try_send(notification) {
                    Ok(()) => {}
                    // 客户端消费不过来：丢这一条，不阻塞内核事件循环。
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

/// 单条入站信封的完整处理：能力响应回填 / hello 注册 / 业务分发。
async fn handle_message(
    state: &Arc<ServerState>,
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
        // 连接协商：服务端自报身份、协议版本与能力，客户端据此校验版本。
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
                client = client_name,
                version = client_version,
                capabilities = ?params.get("capabilities"),
                "initialize"
            );

            // 协议版本不兼容时明确拒绝，而不是带着未知语义继续跑。
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

            // 按结构化能力注册桌面宿主：客户端声明 browser 能力，
            // 内核才会把浏览器自动化请求路由回它。
            let wants_browser = params
                .pointer("/capabilities/browser")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if wants_browser {
                state.hub.register_desktop(out_tx.clone());
                info!("desktop client registered (browser capability)");
            }

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
        // 握手完成通知：客户端确认收到 initialize 响应。无回包。
        "initialized" => {
            info!("client initialized");
        }
        "hello" => {
            let is_desktop = params
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|caps| caps.iter().any(|c| c.as_str() == Some("browser")))
                .unwrap_or(false);
            if is_desktop {
                state.hub.register_desktop(out_tx.clone());
                info!("desktop client registered (browser capability)");
            }
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": true },
            });
            let _ = out_tx.try_send(response);
        }
        _ => {
            // 每个请求独立 spawn：一个长 turn 不得阻塞读循环，
            // 否则取消、权限审批与后续消息全部堵死（Codex 的 submission 模型同此）。
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
            tokio::spawn(async move {
                let _permit = permit;
                let result = dispatch::dispatch(&state.core, method.as_str(), params).await;
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
// stdio 传输（桌面默认）
// ─────────────────────────────────────────────

/// 通过 stdin/stdout 的 JSONL 协议提供服务。
///
/// 父进程（桌面壳）关闭管道即视为会话结束：读到 EOF 立刻退出，
/// 保证不会留下孤儿进程继续占用 SQLite 与 MCP 连接。
pub async fn run_stdio() -> anyhow::Result<()> {
    let (core, hub, _instance_lock) = bootstrap().await?;
    let state = Arc::new(ServerState {
        core,
        hub,
        inflight: Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS)),
    });

    let (out_tx, mut out_rx) = mpsc::channel::<Value>(OUTBOUND_QUEUE_CAPACITY);

    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = out_rx.recv().await {
            let Ok(mut line) = serde_json::to_vec(&message) else {
                continue;
            };
            line.push(b'\n');
            if stdout.write_all(&line).await.is_err() {
                break;
            }
            if stdout.flush().await.is_err() {
                break;
            }
        }
    });

    // stdio 只有一个客户端，启动即注册桌面能力（客户端随后发 hello 亦可，重复注册无害）。
    state.hub.register_desktop(out_tx.clone());
    let pump = tokio::spawn(event_pump(state.core.clone(), out_tx.clone()));

    info!("nova daemon serving over stdio");

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(envelope) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                handle_message(&state, envelope, &out_tx).await;
            }
            Ok(None) => break,
            Err(error) => {
                warn!(%error, "stdio 读取失败，守护进程退出");
                break;
            }
        }
    }

    // 收尾：先摘掉 hub 与本地持有的出站句柄，让 writer 把已排队的响应写完，
    // 再终止事件泵。直接 abort writer 会丢掉 EOF 前最后一条响应。
    pump.abort();
    state.hub.clear_desktop();
    drop(out_tx);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
    info!("nova daemon stdio channel closed, exiting");
    Ok(())
}

// ─────────────────────────────────────────────
// WebSocket 传输（远程 / 可选）
// ─────────────────────────────────────────────

pub async fn run(host_addr: &str, port: u16) -> anyhow::Result<()> {
    let (core, hub, _instance_lock) = bootstrap().await?;
    let state = Arc::new(ServerState {
        core,
        hub,
        inflight: Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS)),
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
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<ServerState>) {
    let (mut sink, mut stream) = socket.split();

    // 出站消息通道：事件扇出、RPC 响应、能力请求统一走这里。
    let (out_tx, mut out_rx) = mpsc::channel::<Value>(OUTBOUND_QUEUE_CAPACITY);
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let text = serde_json::to_string(&message).unwrap_or_default();
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // 内核事件扇出（所有连接共享同一事件流）。
    let pump = tokio::spawn(event_pump(state.core.clone(), out_tx.clone()));

    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            handle_message(&state, envelope, &out_tx).await;
        }
    });

    // 任一方向结束即清理整个连接。
    tokio::select! {
        _ = writer => {},
        _ = pump => {},
        _ = reader => {},
    }
}
