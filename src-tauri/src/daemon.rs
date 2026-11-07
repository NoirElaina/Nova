//! 桌面端 → Nova 内核子进程客户端（桌面薄壳的唯一后端通道）。
//!
//! 传输：本机 stdio 上的 JSONL（每行一个 JSON-RPC 信封），与 Codex App Server 同构。
//! 不走 TCP 的理由不是因为「更先进」，而是：没有监听端口，就没有鉴权面、端口冲突与
//! 单实例争议，子进程还会随管道关闭自行退出，不会留下孤儿。
//!
//! 职责：
//! - 拉起并托管 `nova serve`（stdio 模式），异常退出后退避重启；
//! - IPC 命令经 JSON-RPC 转发（方法名与前端命令一致）；
//! - 内核事件通知原样 re-emit 给前端（事件契约零改动）；
//! - 桌面能力服务端：响应 daemon 的 `capability/browser` 请求与 `capability/hooks` 通知。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};
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

/// 获取当前连接客户端，若未就绪则在指定超时内异步等待内核子进程就绪
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
        Err("Nova 内核通道已关闭".to_string())
    })
    .await
    .map_err(|_| "等待 Nova 内核子进程启动超时，请检查后台进程状态".to_string())?
}

/// 启动内核子进程管理循环：拉起、托管、异常退出后退避重启，直到应用结束。
pub fn init(app: &AppHandle, cli_path: Option<PathBuf>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(cli_path) = cli_path else {
            warn!("未找到可用的 Nova CLI，内核通道不可用");
            return;
        };

        let mut attempt: u32 = 0;
        loop {
            match spawn_child(&cli_path).await {
                Ok(child) => {
                    info!(
                        pid = child.id().unwrap_or_default(),
                        path = %cli_path.display(),
                        "Nova 内核子进程已拉起（stdio）"
                    );
                    attempt = 0;
                    run_connection(child, app.clone()).await;
                    warn!("Nova 内核子进程已退出，准备重启");
                }
                Err(error) => {
                    warn!(%error, "拉起 Nova 内核子进程失败");
                    attempt = attempt.saturating_add(1);
                }
            }

            // 前几次快速重试（冷启动竞态），之后逐步退避，避免坏二进制把 CPU 打满。
            let delay = match attempt {
                0 => Duration::from_millis(300),
                n if n < 5 => Duration::from_millis(500 * u64::from(n)),
                _ => Duration::from_secs(5),
            };
            tokio::time::sleep(delay).await;
        }
    });
}

async fn spawn_child(cli_path: &PathBuf) -> Result<Child, String> {
    let mut command = Command::new(cli_path);
    command
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // 子进程日志走 stderr：stdout 已被协议流独占，绝不能混入。
        .stderr(Stdio::inherit());

    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    // 兜底：即使宿主异常退出、管道未及时关闭，子进程也会随句柄释放被终止。
    command
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn {} 失败: {}", cli_path.display(), e))
}

/// 转发一条 IPC 命令到内核子进程并等待结果。
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
        .map_err(|_| "内核通道已关闭".to_string())?;

    rx.await
        .map_err(|_| "内核响应通道已关闭".to_string())?
}

async fn run_connection(mut child: Child, app: AppHandle) {
    let Some(stdin) = child.stdin.take() else {
        warn!("内核子进程没有可用的 stdin");
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        warn!("内核子进程没有可用的 stdout");
        return;
    };

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

    let client = Arc::new(DaemonClient {
        out: out_tx.clone(),
        pending: pending.clone(),
        counter: AtomicU64::new(0),
    });
    set_active_client(Some(client));
    let _ = app.emit("daemon-connected", ());

    // initialize：声明桌面身份与能力，服务端回包自报版本与协议版本。
    let _ = out_tx.send(json!({
        "jsonrpc": "2.0",
        "id": "initialize",
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "nova_desktop",
                "title": "Nova Desktop",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": ["browser"],
        },
    }));

    let mut writer = tokio::spawn(async move {
        let mut stdin = stdin;
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

    let reader_app = app.clone();
    let reader_pending = pending.clone();
    let reader_out = out_tx.clone();
    let mut reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(envelope) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    dispatch_incoming(&reader_app, &reader_out, &reader_pending, envelope).await;
                }
                // 子进程关闭 stdout：连接结束，外层会重启。
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = &mut writer => {},
        _ = &mut reader => {},
        status = child.wait() => {
            match status {
                Ok(status) => warn!(%status, "Nova 内核子进程退出"),
                Err(error) => warn!(%error, "等待 Nova 内核子进程失败"),
            }
        }
    }

    writer.abort();
    reader.abort();

    set_active_client(None);
    pending.lock().unwrap().clear();
    let _ = app.emit("daemon-disconnected", ());
}

/// 处理一条来自内核子进程的入站消息。
async fn dispatch_incoming(
    app: &AppHandle,
    out_tx: &mpsc::UnboundedSender<Value>,
    pending: &PendingMap,
    envelope: Value,
) {
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
                let _ = app.emit(name, payload);
            }
        }
        // 浏览器能力请求：本地窗口执行后回包。
        "capability/browser" => {
            let id = envelope.get("id").cloned();
            let params = envelope.get("params").cloned().unwrap_or(Value::Null);
            let app = app.clone();
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
                // 握手回包：比对内核版本，不一致时显式告警，
                // 替代「每次启动强制覆写二进制」那种隐式兜底。
                if id == "initialize" {
                    let server = envelope
                        .pointer("/result/serverInfo/version")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let protocol = envelope
                        .pointer("/result/protocolVersion")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let host = env!("CARGO_PKG_VERSION");
                    if !server.is_empty() && server != host {
                        warn!(server, host, "内核版本与桌面壳不一致，需要重新部署 CLI");
                        let _ = app.emit(
                            "kernel-version-mismatch",
                            json!({ "server": server, "host": host, "protocol": protocol }),
                        );
                    } else {
                        info!(server, protocol, "内核握手完成");
                    }
                    return;
                }

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
