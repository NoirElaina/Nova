//! Nova 内核客户端。
//!
//! 唯一职责：拉起 `nova serve`（stdio 模式），把 JSON-RPC 请求写进它的 stdin、
//! 从 stdout 读回响应与事件。**桌面、TUI、无头三种宿主都走这一份实现**，
//! 这样内核只有一条接入路径，不需要在多个宿主里各写一遍。
//!
//! 设计对照 `codex-rs/app-server-client`：本地客户端 spawn 子进程并持有管道，
//! 不走 TCP，因此没有端口、没有鉴权面、没有单实例争议。

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot};

/// 内核返回的 JSON-RPC 错误（保留错误码，重试判断需要）。
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>>;

/// 内核推送的事件：`(事件名, 载荷)`。事件名契约由 `NovaEvent::to_frontend` 定义。
pub type ClientEvent = (String, Value);

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

pub struct Client {
    out: mpsc::Sender<Value>,
    pending: Pending,
    events: broadcast::Sender<ClientEvent>,
    counter: AtomicU64,
    pid: u32,
    /// 必须持有：`kill_on_drop(true)` 的 Child 一旦被 drop，内核子进程会被立刻终止。
    child: Mutex<Child>,
}

impl Client {
    /// 拉起内核子进程并完成 `initialize` 握手（无宿主能力）。
    pub async fn spawn(cli_path: &Path) -> Result<Arc<Self>, String> {
        Self::spawn_with(cli_path, ClientOptions::default()).await
    }

    /// 拉起内核子进程并完成 `initialize` 握手，可注册宿主能力。
    pub async fn spawn_with(
        cli_path: &Path,
        options: ClientOptions,
    ) -> Result<Arc<Self>, String> {
        let mut command = Command::new(cli_path);
        command
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // 内核日志走 stderr：stdout 已被协议流独占。
            .stderr(Stdio::inherit());

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        command.kill_on_drop(true);
        let mut child: Child = command
            .spawn()
            .map_err(|e| format!("spawn {} 失败: {}", cli_path.display(), e))?;

        let pid = child.id().unwrap_or(0);
        let mut stdin = child.stdin.take().ok_or("内核子进程缺少 stdin")?;
        let stdout = child.stdout.take().ok_or("内核子进程缺少 stdout")?;

        let (out_tx, mut out_rx) = mpsc::channel::<Value>(1024);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        // 逐 token 的 delta 事件很密集，容量偏小时容易触发 Lagged 丢事件。
        let (event_tx, _) = broadcast::channel::<ClientEvent>(8192);

        tokio::spawn(async move {
            while let Some(message) = out_rx.recv().await {
                let Ok(mut line) = serde_json::to_vec(&message) else {
                    continue;
                };
                line.push(b'\n');
                if stdin.write_all(&line).await.is_err() {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        let reader_pending = pending.clone();
        let reader_events = event_tx.clone();
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
                    &reader_events,
                    reader_browser.as_ref(),
                    reader_hooks.as_ref(),
                    envelope,
                )
                .await;
            }
        });

        let client = Arc::new(Self {
            out: out_tx,
            pending,
            events: event_tx,
            counter: AtomicU64::new(0),
            pid,
            child: Mutex::new(child),
        });

        // 握手：按注册的宿主能力声明 capabilities（桌面 browser=true，CLI 为 false）。
        let capabilities = json!({
            "browser": options.browser.is_some(),
            "hooks": options.hooks.is_some(),
        });
        let _ = client
            .out
            .send(json!({
                "jsonrpc": "2.0",
                "id": "initialize",
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "nova_client",
                        "title": "Nova Client",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": capabilities,
                    "protocolVersion": nova_protocol::rpc::PROTOCOL_VERSION,
                },
            }))
            .await;

        // 握手确认通知（对齐 Codex 的 initialized）。
        let _ = client
            .out
            .send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }))
            .await;

        Ok(client)
    }

    /// 发起一次 RPC 并等待结果。
    ///
    /// 收到 `-32001`（过载）时按指数退避 + jitter 重试，最多 5 次——
    /// 与 Codex 对 "Server overloaded; retry later." 的处理一致。
    pub async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        const MAX_RETRIES: u32 = 5;
        let mut attempt: u32 = 0;
        loop {
            match self.invoke_once(method, params.clone()).await {
                Ok(value) => return Ok(value),
                Err(RpcError { code: -32001, .. }) if attempt < MAX_RETRIES => {
                    let base = 100u64.saturating_mul(1u64 << attempt.min(6));
                    let jitter = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| u64::from(d.subsec_nanos()) % 100)
                        .unwrap_or(0);
                    tokio::time::sleep(std::time::Duration::from_millis(base + jitter)).await;
                    attempt += 1;
                }
                Err(error) => return Err(error.message),
            }
        }
    }

    async fn invoke_once(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = format!("rpc-{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        self.out
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await
            .map_err(|_| RpcError {
                code: -32000,
                message: "内核通道已关闭".to_string(),
            })?;

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RpcError {
                code: -32000,
                message: "内核响应通道已关闭".to_string(),
            }),
            Err(_) => Err(RpcError {
                code: -32000,
                message: format!("调用 '{}' 超时（内核可能未就绪）", method),
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.events.subscribe()
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// 显式结束内核子进程。不调用也可以：Client 被 drop 时 Child 会一并终止。
    pub fn shutdown(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }
}

async fn dispatch(
    out: &mpsc::Sender<Value>,
    pending: &Pending,
    events: &broadcast::Sender<ClientEvent>,
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
            let _ = events.send((name, payload));
        }
        // 浏览器自动化：桌面注册了能力就本地执行后回包；CLI 宿主明确回绝，
        // 避免内核等到 120 秒超时。
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
        // 宿主钩子：火忘式通知，无回包。
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
                if let Some(tx) = pending.lock().unwrap().remove(id) {
                    let _ = tx.send(payload);
                }
            }
        }
    }
}
