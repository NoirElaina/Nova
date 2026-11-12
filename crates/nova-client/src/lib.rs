//! Nova 内核客户端。
//!
//! 唯一职责：连接或拉起 `nova serve`，经 JSON-RPC 收发请求、事件与宿主能力回调。
//! **桌面、TUI、无头三种宿主都走这一份实现**，提供两种传输形态：
//! - `spawn` / `spawn_with`：拉起独占 stdio 子进程（用于 `-p` 无头、TUI、管道测试）；
//! - `connect_or_spawn`：多窗口共享守护进程（WebSocket over UDS，含 probe-first、版本握手与安全校验）。

pub mod uds;

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fs2::FileExt;
use futures_util::{SinkExt, StreamExt};
use nova_protocol::rpc::{InitializeResult, PROTOCOL_VERSION};
use nova_protocol::sync::MutexExt;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// 内核返回的 JSON-RPC 错误（保留错误码，重试判断需要）。
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>>;

/// 内核推送的事件：`(事件名, 载荷)`。事件名契约由 `NovaEvent::to_frontend` 定义。
pub type ClientEvent = (String, Value);

/// 单个事件订阅端队列上限（防止慢消费端无限堆积内存导致 OOM）。
pub const SUBSCRIBER_QUEUE_CAPACITY: usize = 4096;

struct SubscriberSlot {
    tx: mpsc::Sender<ClientEvent>,
    dropped: AtomicU64,
}

impl SubscriberSlot {
    fn new(tx: mpsc::Sender<ClientEvent>) -> Self {
        Self {
            tx,
            dropped: AtomicU64::new(0),
        }
    }
}

type Subscribers = Arc<Mutex<Vec<SubscriberSlot>>>;

/// 宿主能力：处理内核发来的浏览器自动化请求，返回回包。
pub type BrowserHandler = Arc<
    dyn Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send>>
        + Send
        + Sync,
>;

/// 宿主钩子：处理内核下发的通知（火忘式，无回包）。
pub type HookHandler = Arc<dyn Fn(String, Value) + Send + Sync>;

/// 客户端启动选项。桌面注册能力，CLI 宿主留空。
#[derive(Clone, Default)]
pub struct ClientOptions {
    pub browser: Option<BrowserHandler>,
    pub hooks: Option<HookHandler>,
}

enum ClientBackend {
    StdioChild {
        shutdown_tx: Mutex<Option<oneshot::Sender<String>>>,
        child: Box<Mutex<Child>>,
    },
    SharedSocket {
        close_tx: Mutex<Option<oneshot::Sender<()>>>,
    },
}

enum ProbeError {
    NotRunning(String),
    IncompatibleVersion { client: Arc<Client>, detail: String },
    Fatal(String),
}

pub struct Client {
    out: mpsc::Sender<Value>,
    pending: Pending,
    subscribers: Subscribers,
    counter: AtomicU64,
    pid: u32,
    shutting_down: AtomicBool,
    backend: ClientBackend,
}

impl Client {
    /// 拉起独占内核子进程并完成 `initialize` 握手（无宿主能力）。
    pub async fn spawn(cli_path: &Path) -> Result<Arc<Self>, String> {
        Self::spawn_with(cli_path, ClientOptions::default()).await
    }

    /// 拉起独占内核子进程（stdio JSONL）并完成 `initialize` 握手，可注册宿主能力。
    pub async fn spawn_with(
        cli_path: &Path,
        options: ClientOptions,
    ) -> Result<Arc<Self>, String> {
        let mut command = Command::new(cli_path);
        command
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        apply_windows_no_window(&mut command);
        command.kill_on_drop(true);

        let mut child: Child = command
            .spawn()
            .map_err(|e| format!("spawn {} 失败: {}", cli_path.display(), e))?;

        let pid = child.id().unwrap_or(0);
        let mut stdin = child.stdin.take().ok_or("内核子进程缺少 stdin")?;
        let stdout = child.stdout.take().ok_or("内核子进程缺少 stdout")?;

        let (out_tx, mut out_rx) = mpsc::channel::<Value>(1024);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<String>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));

        tokio::spawn(async move {
            let mut shutdown_rx = Some(shutdown_rx);
            loop {
                tokio::select! {
                    biased;
                    line = async {
                        match shutdown_rx.as_mut() {
                            Some(rx) => rx.await,
                            None => std::future::pending().await,
                        }
                    }, if shutdown_rx.is_some() => {
                        if let Ok(shutdown_line) = line {
                            let _ = stdin.write_all(shutdown_line.as_bytes()).await;
                            let _ = stdin.flush().await;
                        }
                        break;
                    }
                    msg = out_rx.recv() => {
                        let Some(message) = msg else { break; };
                        let Ok(mut line) = serde_json::to_vec(&message) else {
                            continue;
                        };
                        line.push(b'\n');
                        if stdin.write_all(&line).await.is_err() || stdin.flush().await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let reader_pending = pending.clone();
        let reader_subscribers = subscribers.clone();
        let reader_out = out_tx.clone();
        let reader_browser = options.browser.clone();
        let reader_hooks = options.hooks.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(envelope) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                dispatch(
                    &reader_out,
                    &reader_pending,
                    &reader_subscribers,
                    reader_browser.as_ref(),
                    reader_hooks.as_ref(),
                    envelope,
                )
                .await;
            }
            cleanup_client_channels(&reader_pending, &reader_subscribers);
        });

        let client = Arc::new(Self {
            out: out_tx,
            pending,
            subscribers,
            counter: AtomicU64::new(0),
            pid,
            shutting_down: AtomicBool::new(false),
            backend: ClientBackend::StdioChild {
                shutdown_tx: Mutex::new(Some(shutdown_tx)),
                child: Box::new(Mutex::new(child)),
            },
        });

        send_initialize_and_ack(&client, &options).await?;
        Ok(client)
    }

    /// 仅连接已存在的共享 UDS 守护进程并完成版本握手（不自动拉起）。
    pub async fn connect_uds(socket_path: &Path, options: ClientOptions) -> Result<Arc<Self>, String> {
        match try_connect_uds(socket_path, &options).await {
            Ok(client) => Ok(client),
            Err(ProbeError::IncompatibleVersion { client, detail }) => {
                drop(client);
                Err(detail)
            }
            Err(ProbeError::NotRunning(err) | ProbeError::Fatal(err)) => Err(err),
        }
    }

    /// Probe-first 连接共享 UDS 守护进程；仅在未运行或版本不兼容时加启动锁拉起新实例。
    pub async fn connect_or_spawn(
        cli_path: &Path,
        socket_path: &Path,
        options: ClientOptions,
    ) -> Result<Arc<Self>, String> {
        match try_connect_uds(socket_path, &options).await {
            Ok(client) => return Ok(client),
            Err(ProbeError::Fatal(err)) => return Err(err),
            Err(ProbeError::IncompatibleVersion { client, detail }) => {
                tracing::warn!(%detail, "stale daemon version detected; requesting graceful shutdown");
                shutdown_stale_daemon(&client, socket_path, &detail).await?;
            }
            Err(ProbeError::NotRunning(_)) => {}
        }

        let socket_dir = socket_path
            .parent()
            .ok_or_else(|| format!("无效守护进程套接字路径: {}", socket_path.display()))?;
        uds::prepare_private_socket_directory(socket_dir)
            .await
            .map_err(|e| format!("初始化私有套接字目录失败: {e}"))?;

        let lock_path = socket_dir.join("nova.startup.lock");
        let _startup_lock = acquire_startup_lock(lock_path).await?;

        // 双重检查：持锁期间可能已由另一扇窗口完成启动。
        match try_connect_uds(socket_path, &options).await {
            Ok(client) => return Ok(client),
            Err(ProbeError::Fatal(err)) => return Err(err),
            Err(ProbeError::IncompatibleVersion { client, detail }) => {
                tracing::warn!(%detail, "stale daemon still present under lock; shutting down");
                shutdown_stale_daemon(&client, socket_path, &detail).await?;
            }
            Err(ProbeError::NotRunning(_)) => {}
        }

        let _ = tokio::fs::remove_file(socket_path).await;

        let mut command = Command::new(cli_path);
        command
            .arg("serve")
            .arg("--uds")
            .arg(socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        apply_windows_no_window(&mut command);

        let mut child = command
            .spawn()
            .map_err(|e| format!("拉起共享内核守护进程失败 ({}): {e}", cli_path.display()))?;

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut backoff = Duration::from_millis(25);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "共享内核守护进程在就绪前退出（状态: {status}）"
                ));
            }
            match try_connect_uds(socket_path, &options).await {
                Ok(client) => return Ok(client),
                Err(ProbeError::Fatal(err)) => return Err(err),
                Err(ProbeError::IncompatibleVersion { detail, .. }) => {
                    return Err(format!("刚拉起的守护进程版本仍不兼容: {detail}"));
                }
                Err(ProbeError::NotRunning(err)) => {
                    if Instant::now() >= deadline {
                        return Err(format!("等待共享守护进程就绪超时: {err}"));
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_millis(200));
                }
            }
        }
    }

    /// 发起一次 RPC 并等待结果。收到 `-32001`（过载）时按指数退避 + jitter 重试，最多 5 次。
    pub async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        const MAX_RETRIES: u32 = 5;
        let mut attempt: u32 = 0;
        loop {
            match self.invoke_once(method, params.clone(), Duration::from_secs(30)).await {
                Ok(value) => return Ok(value),
                Err(RpcError { code: -32001, .. }) if attempt < MAX_RETRIES => {
                    let base = 100u64.saturating_mul(1u64 << attempt.min(6));
                    let jitter = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| u64::from(d.subsec_nanos()) % 100)
                        .unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis(base + jitter)).await;
                    attempt += 1;
                }
                Err(error) => return Err(error.message),
            }
        }
    }

    async fn invoke_once(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let id = format!("rpc-{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock_unpoisoned().insert(id.clone(), tx);
        if self
            .out
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await
            .is_err()
        {
            self.pending.lock_unpoisoned().remove(&id);
            return Err(RpcError {
                code: -32000,
                message: "内核通道已关闭".to_string(),
            });
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RpcError {
                code: -32000,
                message: "内核响应通道已关闭".to_string(),
            }),
            Err(_) => {
                self.pending.lock_unpoisoned().remove(&id);
                Err(RpcError {
                    code: -32000,
                    message: format!("调用 '{}' 超时（内核可能未就绪）", method),
                })
            }
        }
    }

    pub fn subscribe(&self) -> mpsc::Receiver<ClientEvent> {
        let (tx, rx) = mpsc::channel(SUBSCRIBER_QUEUE_CAPACITY);
        self.subscribers
            .lock_unpoisoned()
            .push(SubscriberSlot::new(tx));
        rx
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// 向共享守护进程发送 `shutdown` RPC，显式请求整进程退出（如升级不兼容旧版或测试收尾）。
    pub async fn shutdown_daemon(&self) -> Result<(), String> {
        let _ = self
            .invoke_once("shutdown", json!({}), Duration::from_secs(3))
            .await
            .map_err(|e| e.message)?;
        self.shutdown();
        Ok(())
    }

    /// 结束当前客户端连接。
    ///
    /// - `StdioChild`：发送 `shutdown` RPC 并回收独占子进程；
    /// - `SharedSocket`：仅关闭当前窗口的 WebSocket 连接，不误杀其他窗口共享的守护进程。
    pub fn shutdown(&self) {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            return;
        }
        match &self.backend {
            ClientBackend::StdioChild { shutdown_tx, child } => {
                let request_id = format!("shutdown-{}", self.counter.fetch_add(1, Ordering::Relaxed));
                let payload = json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "shutdown",
                    "params": {},
                });

                let sent_priority = shutdown_tx
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take())
                    .and_then(|tx| {
                        serde_json::to_string(&payload)
                            .ok()
                            .map(|mut s| {
                                s.push('\n');
                                tx.send(s).is_ok()
                            })
                    })
                    .unwrap_or(false);
                let graceful = sent_priority || self.out.try_send(payload).is_ok();

                if let Ok(mut child) = child.lock() {
                    if graceful {
                        let deadline = Instant::now() + Duration::from_millis(2500);
                        while Instant::now() < deadline {
                            match child.try_wait() {
                                Ok(Some(_)) => return,
                                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                                Err(_) => break,
                            }
                        }
                    }
                    let _ = child.start_kill();
                }
            }
            ClientBackend::SharedSocket { close_tx } => {
                if let Ok(mut guard) = close_tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(());
                    }
                }
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn apply_windows_no_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

async fn send_initialize_and_ack(
    client: &Client,
    options: &ClientOptions,
) -> Result<InitializeResult, String> {
    let result_val = client
        .invoke_once(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "nova_client",
                    "title": "Nova Client",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "browser": options.browser.is_some(),
                    "hooks": options.hooks.is_some(),
                },
                "protocolVersion": PROTOCOL_VERSION,
            }),
            Duration::from_secs(5),
        )
        .await
        .map_err(|e| format!("initialize 握手失败: {}", e.message))?;

    let init: InitializeResult = serde_json::from_value(result_val)
        .map_err(|e| format!("initialize 响应格式不兼容: {e}"))?;

    Ok(init)
}

async fn ack_initialized(client: &Client) -> Result<(), String> {
    client
        .out
        .send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }))
        .await
        .map_err(|_| "发送 initialized 通知失败：连接已关闭".to_string())
}

async fn try_connect_uds(socket_path: &Path, options: &ClientOptions) -> Result<Arc<Client>, ProbeError> {
    let (validated_path, _dir_guard) = match uds::validate_private_socket_path(socket_path) {
        Ok(ok) => ok,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProbeError::NotRunning(err.to_string()));
        }
        Err(err) => {
            return Err(ProbeError::Fatal(format!(
                "守护进程套接字目录校验失败 ({}): {err}",
                socket_path.display()
            )));
        }
    };

    let stream = uds::UnixStream::connect(&validated_path).await.map_err(|err| {
        if is_connect_not_running(&err) {
            ProbeError::NotRunning(err.to_string())
        } else {
            ProbeError::Fatal(format!("连接 UDS 失败 ({}): {err}", socket_path.display()))
        }
    })?;

    stream.ensure_trusted_peer().map_err(|err| {
        ProbeError::Fatal(format!("UDS 对端身份安全校验失败: {err}"))
    })?;

    let (ws_stream, _) = tokio_tungstenite::client_async("ws://localhost/ws", stream)
        .await
        .map_err(|err| ProbeError::NotRunning(format!("WebSocket over UDS 握手失败: {err}")))?;

    let (mut sink, mut rx_stream) = ws_stream.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Value>(1024);
    let (close_tx, close_rx) = oneshot::channel::<()>();
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));

    tokio::spawn(async move {
        let mut close_rx = Some(close_rx);
        loop {
            tokio::select! {
                biased;
                _ = async {
                    match close_rx.as_mut() {
                        Some(rx) => rx.await,
                        None => std::future::pending().await,
                    }
                }, if close_rx.is_some() => {
                    let _ = sink.send(WsMessage::Close(None)).await;
                    let _ = sink.close().await;
                    break;
                }
                msg = out_rx.recv() => {
                    let Some(message) = msg else { break; };
                    let Ok(text) = serde_json::to_string(&message) else {
                        continue;
                    };
                    if sink.send(WsMessage::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let reader_pending = pending.clone();
    let reader_subscribers = subscribers.clone();
    let reader_out = out_tx.clone();
    let reader_browser = options.browser.clone();
    let reader_hooks = options.hooks.clone();
    tokio::spawn(async move {
        while let Some(Ok(message)) = rx_stream.next().await {
            let WsMessage::Text(text) = message else {
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            dispatch(
                &reader_out,
                &reader_pending,
                &reader_subscribers,
                reader_browser.as_ref(),
                reader_hooks.as_ref(),
                envelope,
            )
            .await;
        }
        cleanup_client_channels(&reader_pending, &reader_subscribers);
    });

    let client = Arc::new(Client {
        out: out_tx,
        pending,
        subscribers,
        counter: AtomicU64::new(0),
        pid: 0,
        shutting_down: AtomicBool::new(false),
        backend: ClientBackend::SharedSocket {
            close_tx: Mutex::new(Some(close_tx)),
        },
    });

    let init = match send_initialize_and_ack(&client, options).await {
        Ok(init) => init,
        Err(err) => {
            client.shutdown();
            return Err(ProbeError::NotRunning(err));
        }
    };

    let expected_client_version = env!("CARGO_PKG_VERSION");
    if init.protocol_version != PROTOCOL_VERSION || init.server_info.version != expected_client_version
    {
        return Err(ProbeError::IncompatibleVersion {
            client,
            detail: format!(
                "协议或版本不匹配: daemon(protocol={}, version={}) != client(protocol={}, version={})",
                init.protocol_version,
                init.server_info.version,
                PROTOCOL_VERSION,
                expected_client_version
            ),
        });
    }

    if let Err(err) = ack_initialized(&client).await {
        client.shutdown();
        return Err(ProbeError::NotRunning(err));
    }

    Ok(client)
}

async fn shutdown_stale_daemon(
    client: &Client,
    socket_path: &Path,
    detail: &str,
) -> Result<(), String> {
    if let Err(err) = client.shutdown_daemon().await {
        return Err(format!("{detail}；尝试关闭旧 daemon 失败: {err}"));
    }

    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match uds::UnixStream::connect(socket_path).await {
            Err(err) if is_connect_not_running(&err) => return Ok(()),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    Err(format!("{detail}；旧 daemon 未在规定时间内释放套接字"))
}

fn is_connect_not_running(err: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        err.kind(),
        ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
    )
}

async fn acquire_startup_lock(lock_path: std::path::PathBuf) -> Result<std::fs::File, String> {
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| format!("无法打开启动锁 {}: {e}", lock_path.display()))?;
        file.lock_exclusive()
            .map_err(|e| format!("获取启动锁失败 {}: {e}", lock_path.display()))?;
        Ok(file)
    })
    .await
    .map_err(|e| format!("启动锁任务异常: {e}"))?
}

fn cleanup_client_channels(pending: &Pending, subscribers: &Subscribers) {
    pending.lock_unpoisoned().clear();
    subscribers.lock_unpoisoned().clear();
}

async fn dispatch(
    out: &mpsc::Sender<Value>,
    pending: &Pending,
    subscribers: &Subscribers,
    browser: Option<&BrowserHandler>,
    hooks: Option<&HookHandler>,
    envelope: Value,
) {
    match envelope.get("method").and_then(Value::as_str) {
        Some("event") => {
            let name = envelope
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let payload = envelope
                .pointer("/params/payload")
                .cloned()
                .unwrap_or(Value::Null);
            let event = (name, payload);
            let mut subs = subscribers.lock_unpoisoned();
            subs.retain(|slot| match slot.tx.try_send(event.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let count = slot.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                    if count == 1 || count % 100 == 0 {
                        tracing::warn!(
                            dropped = count,
                            capacity = SUBSCRIBER_QUEUE_CAPACITY,
                            event = %event.0,
                            "nova-client subscriber event queue full; dropping event due to backpressure"
                        );
                    }
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            });
        }
        Some("capability/browser") => {
            let id = envelope.get("id").cloned();
            let params = envelope.get("params").cloned().unwrap_or(Value::Null);
            match browser {
                Some(handler) => {
                    let handler = handler.clone();
                    let out = out.clone();
                    tokio::spawn(async move {
                        let result = handler(params).await;
                        let _ = out
                            .send(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
                            .await;
                    });
                }
                None => {
                    let _ = out
                        .send(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "ok": false, "error": "当前宿主不支持浏览器自动化" },
                        }))
                        .await;
                }
            }
        }
        Some("capability/hooks") => {
            if let Some(handler) = hooks {
                let params = envelope.get("params").cloned().unwrap_or(Value::Null);
                let operation = params
                    .get("operation")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args = params.get("params").cloned().unwrap_or(Value::Null);
                handler(operation, args);
            }
        }
        _ => {
            if let Some(id) = envelope.get("id").and_then(Value::as_str) {
                let payload = if let Some(error) = envelope.get("error") {
                    Err(RpcError {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603) as i32,
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("daemon error")
                            .to_string(),
                    })
                } else {
                    Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
                };
                if let Some(tx) = pending.lock_unpoisoned().remove(id) {
                    let _ = tx.send(payload);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bounded_event_delivery_within_capacity() {
        let (out_tx, _) = mpsc::channel(1);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));

        let (sub_tx, mut sub_rx) = mpsc::channel(SUBSCRIBER_QUEUE_CAPACITY);
        subscribers
            .lock_unpoisoned()
            .push(SubscriberSlot::new(sub_tx));

        let burst = 2_000usize;
        for i in 0..burst {
            let envelope = json!({
                "method": "event",
                "params": {
                    "name": "agent-event",
                    "payload": { "token": i }
                }
            });
            dispatch(&out_tx, &pending, &subscribers, None, None, envelope).await;
        }

        for i in 0..burst {
            let (name, payload) = sub_rx.recv().await.expect("event must be delivered");
            assert_eq!(name, "agent-event");
            assert_eq!(payload["token"], i);
        }
    }

    #[tokio::test]
    async fn test_slow_subscriber_triggers_backpressure_without_blocking() {
        let (out_tx, _) = mpsc::channel(1);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));

        let (sub_tx, mut sub_rx) = mpsc::channel(4);
        subscribers
            .lock_unpoisoned()
            .push(SubscriberSlot::new(sub_tx));

        for i in 0..10 {
            let envelope = json!({
                "method": "event",
                "params": {
                    "name": "agent-event",
                    "payload": { "token": i }
                }
            });
            dispatch(&out_tx, &pending, &subscribers, None, None, envelope).await;
        }

        for i in 0..4 {
            let (_, payload) = sub_rx.recv().await.expect("buffered event");
            assert_eq!(payload["token"], i);
        }
        assert!(sub_rx.try_recv().is_err());

        let guard = subscribers.lock_unpoisoned();
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].dropped.load(Ordering::Relaxed), 6);
    }

    #[tokio::test]
    async fn test_closed_subscriber_pruned_automatically() {
        let (out_tx, _) = mpsc::channel(1);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));

        let (sub1_tx, sub1_rx) = mpsc::channel(8);
        let (sub2_tx, mut sub2_rx) = mpsc::channel(8);
        subscribers
            .lock_unpoisoned()
            .push(SubscriberSlot::new(sub1_tx));
        subscribers
            .lock_unpoisoned()
            .push(SubscriberSlot::new(sub2_tx));

        drop(sub1_rx);

        let envelope = json!({
            "method": "event",
            "params": {
                "name": "turn_finished",
                "payload": {}
            }
        });
        dispatch(&out_tx, &pending, &subscribers, None, None, envelope).await;

        let (name, _) = sub2_rx.recv().await.expect("sub2 must receive");
        assert_eq!(name, "turn_finished");
        assert_eq!(subscribers.lock_unpoisoned().len(), 1);
    }
}
