//! 桌面端 → Nova 守护进程客户端（桌面薄壳的唯一后端通道）。
//!
//! 职责：
//! - 连接/重连 `ws://127.0.0.1:3080/ws`（daemon 由 supervisor 拉起）；
//! - IPC 命令经 JSON-RPC 转发（方法名与前端命令一致）；
//! - 内核事件通知原样 re-emit 给前端（事件契约零改动）；
//! - 桌面能力服务端：响应 daemon 的 `capability/browser` 请求与 `capability/hooks` 通知。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::connect_async;
use tracing::{info, warn};

type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>;

pub struct DaemonClient {
    out: mpsc::UnboundedSender<Value>,
    pending: PendingMap,
    counter: AtomicU64,
}

struct DaemonChannelState {
    tx: watch::Sender<Option<Arc<DaemonClient>>>,
    rx: watch::Receiver<Option<Arc<DaemonClient>>>,
}

static DAEMON_STATE: OnceLock<DaemonChannelState> = OnceLock::new();

fn get_daemon_state() -> &'static DaemonChannelState {
    DAEMON_STATE.get_or_init(|| {
        let (tx, rx) = watch::channel(None);
        DaemonChannelState { tx, rx }
    })
}

fn set_active_client(client: Option<Arc<DaemonClient>>) {
    get_daemon_state().tx.send_replace(client);
}

impl DaemonClient {
    pub fn get() -> Option<Arc<DaemonClient>> {
        get_daemon_state().rx.borrow().clone()
    }
}

/// 获取当前连接客户端，若未就绪则在指定超时内异步等待守护进程就绪
pub async fn get_or_wait_client(timeout: Duration) -> Result<Arc<DaemonClient>, String> {
    let state = get_daemon_state();
    if let Some(client) = state.rx.borrow().clone() {
        return Ok(client);
    }

    let mut rx = state.rx.clone();
    tokio::time::timeout(timeout, async {
        if let Some(client) = rx.borrow().clone() {
            return Ok(client);
        }
        while rx.changed().await.is_ok() {
            if let Some(client) = rx.borrow().clone() {
                return Ok(client);
            }
        }
        Err("Nova 守护进程连接通道已关闭".to_string())
    })
    .await
    .map_err(|_| "等待 Nova 后台服务启动就绪超时，请检查后台守护进程状态".to_string())?
}

/// 启动连接管理循环：自动重连直到 daemon 可用，断线后继续重连。
pub fn init(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut attempt: u32 = 0;
        loop {
            match connect_async("ws://127.0.0.1:3080/ws").await {
                Ok((socket, _)) => {
                    info!("connected to nova daemon");
                    attempt = 0;
                    run_connection(socket, app.clone()).await;
                    warn!("nova daemon connection lost, retrying...");
                }
                Err(error) => {
                    warn!(%error, "nova daemon unavailable, retrying...");
                    attempt = attempt.saturating_add(1);
                }
            }
            // 启动前 20 次尝试以 100ms 快速探测，缩短就绪等待时间；后续保持 500ms 探测
            let delay = if attempt < 20 {
                Duration::from_millis(100)
            } else {
                Duration::from_millis(500)
            };
            tokio::time::sleep(delay).await;
        }
    });
}

/// 转发一条 IPC 命令到 daemon 并等待结果。
pub async fn invoke(method: &str, params: Value) -> Result<Value, String> {
    let client = get_or_wait_client(Duration::from_secs(10)).await?;
    let id = format!("rpc-{}", client.counter.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = oneshot::channel();
    client.pending.lock().unwrap().insert(id.clone(), tx);

    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    client
        .out
        .send(request)
        .map_err(|_| "daemon channel closed".to_string())?;

    rx.await
        .map_err(|_| "daemon response channel closed".to_string())?
}

async fn run_connection(
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    app: AppHandle,
) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

    let client = Arc::new(DaemonClient {
        out: out_tx.clone(),
        pending: pending.clone(),
        counter: AtomicU64::new(0),
    });
    // 每次重连更新全局客户端并通知前端
    set_active_client(Some(client));
    let _ = app.emit("daemon-connected", ());

    // 注册桌面能力（browser）→ daemon 将把浏览器能力请求路由到本进程。
    let hello = json!({
        "jsonrpc": "2.0",
        "id": "hello",
        "method": "hello",
        "params": { "client": "desktop", "capabilities": ["browser"] },
    });
    let _ = out_tx.send(hello);

    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let text = serde_json::to_string(&message).unwrap_or_default();
            if sink
                .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let reader_app = app.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
                continue;
            };

            let method = envelope.get("method").and_then(Value::as_str).unwrap_or("");
            match method {
                // 内核事件：原样转发给前端。
                "event" => {
                    let name = envelope
                        .pointer("/params/name")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let payload = envelope
                        .pointer("/params/payload")
                        .cloned()
                        .unwrap_or(Value::Null);
                    if !name.is_empty() {
                        let _ = reader_app.emit(name, payload);
                    }
                }
                // 浏览器能力请求：本地窗口执行后回包。
                "capability/browser" => {
                    let id = envelope.get("id").cloned();
                    let params = envelope.get("params").cloned().unwrap_or(Value::Null);
                    let app = reader_app.clone();
                    let out_tx = out_tx.clone();
                    tauri::async_runtime::spawn(async move {
                        let result = crate::services::browser_sessions::run_command(
                            &app,
                            params
                                .get("conversationId")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty()),
                            params.get("action").and_then(Value::as_str).unwrap_or(""),
                            params.get("payload").cloned().unwrap_or(Value::Null),
                            params.get("timeoutMs").and_then(Value::as_u64),
                        )
                        .await;
                        let _ = out_tx.send(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": result,
                        }));
                    });
                }
                // 宿主钩子通知（用户终端清理 / 文件日志开关）。
                "capability/hooks" => {
                    let params = envelope.get("params").cloned().unwrap_or(Value::Null);
                    let operation = params.get("operation").and_then(Value::as_str).unwrap_or("");
                    let args = params.get("params").cloned().unwrap_or(Value::Null);
                    match operation {
                        "stop_user_terminal_session" => {
                            if let Some(id) = args.get("conversationId").and_then(Value::as_str) {
                                let _ = crate::services::user_terminal::stop_session(Some(id));
                            }
                        }
                        "close_all_user_terminal_sessions" => {
                            crate::services::user_terminal::close_all_sessions();
                        }
                        "set_file_logging_enabled" => {
                            let enabled = args
                                .get("enabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            crate::logging::set_file_logging_enabled(enabled);
                        }
                        _ => {}
                    }
                }
                // RPC 响应：按 id 唤醒等待中的请求。
                _ => {
                    if let Some(id) = envelope.get("id").and_then(Value::as_str) {
                        let payload = if let Some(error) = envelope.get("error") {
                            Err(error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("daemon error")
                                .to_string())
                        } else {
                            Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
                        };
                        if let Some(tx) = pending.lock().unwrap().remove(id) {
                            let _ = tx.send(payload);
                        }
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = writer => {},
        _ = reader => {},
    }

    set_active_client(None);
    let _ = app.emit("daemon-disconnected", ());
}
