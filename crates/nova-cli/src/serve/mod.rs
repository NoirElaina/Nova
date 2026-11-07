//! 守护进程：内核经 WebSocket JSON-RPC 对外开放。
//!
//! - 客户端请求：`{jsonrpc, id, method, params}` → 唯一分发器 → `{id, result|error}`；
//! - 内核事件：以 `{method: "event", params: {name, payload}}` 通知扇出（前端契约由内核 to_frontend 定义）；
//! - 桌面能力：浏览器自动化经 `capability/browser` 服务端→客户端请求闭环；
//!   宿主钩子经 `capability/hooks` 通知下发（火忘式）。

mod dispatch;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    response::IntoResponse,
    routing::get,
    Router,
};
use nova_core::{BrowserCapability, Core, HostHooks};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::host;

pub async fn run(host_addr: &str, port: u16) -> anyhow::Result<()> {
    let hub = ClientHub::default();
    let core = host::build_core()?
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

    let state = Arc::new(ServerState { core, hub });
    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((host_addr, port)).await?;
    let addr = listener.local_addr()?;
    info!("Nova daemon listening on {}", addr);
    println!("Nova Daemon 已就绪: http://{} (WebSocket: ws://{}/ws)", addr, addr);
    axum::serve(listener, app).await?;
    Ok(())
}

struct ServerState {
    core: Core,
    hub: ClientHub,
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
    desktop: Option<mpsc::UnboundedSender<Value>>,
    pending: HashMap<String, oneshot::Sender<Value>>,
    counter: u64,
}

impl ClientHub {
    fn register_desktop(&self, sender: mpsc::UnboundedSender<Value>) {
        self.inner.lock().unwrap().desktop = Some(sender);
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
            .send(message)
            .map_err(|_| "desktop client channel closed".to_string())
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
// WebSocket 连接处理
// ─────────────────────────────────────────────

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<ServerState>) {
    let (mut sink, mut stream) = socket.split();

    // 出站消息通道：事件扇出、RPC 响应、能力请求统一走这里。
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let text = serde_json::to_string(&message).unwrap_or_default();
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // 内核事件扇出（所有连接共享同一事件流）。
    let mut events = state.core.subscribe();
    let event_tx = out_tx.clone();
    let event_pump = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let (name, payload) = event.to_frontend();
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "event",
                        "params": { "name": name, "payload": payload },
                    });
                    if event_tx.send(notification).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let core = state.core.clone();
    let hub = state.hub.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            let Message::Text(text) = message else { continue };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else { continue };

            // 能力请求响应（无 method，携带 id + result/error）。
            if envelope.get("method").is_none() {
                if let Some(id) = envelope.get("id").and_then(Value::as_str) {
                    let payload = envelope
                        .get("result")
                        .cloned()
                        .unwrap_or_else(|| envelope.get("error").cloned().unwrap_or(Value::Null));
                    hub.resolve(id, payload);
                }
                continue;
            }

            let method = envelope.get("method").and_then(Value::as_str).unwrap_or("");
            let id = envelope.get("id").cloned();
            let params = envelope.get("params").cloned().unwrap_or(Value::Null);

            match method {
                "hello" => {
                    let is_desktop = params
                        .get("capabilities")
                        .and_then(Value::as_array)
                        .map(|caps| caps.iter().any(|c| c.as_str() == Some("browser")))
                        .unwrap_or(false);
                    if is_desktop {
                        hub.register_desktop(out_tx.clone());
                        info!("desktop client registered (browser capability)");
                    }
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": true },
                    });
                    let _ = out_tx.send(response);
                }
                _ => {
                    let result = dispatch::dispatch(&core, method, params).await;
                    let response = match result {
                        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
                        Err(error) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32000, "message": error },
                        }),
                    };
                    let _ = out_tx.send(response);
                }
            }
        }
    });

    // 任一方向结束即清理整个连接。
    tokio::select! {
        _ = writer => {},
        _ = event_pump => {},
        _ = reader => {},
    }
}
