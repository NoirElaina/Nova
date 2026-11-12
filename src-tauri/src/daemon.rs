//! 桌面端 → 内核子进程客户端（薄壳侧）。
//!
//! 复用 `nova-client`，这里只做两件事：
//! 1. 注册桌面宿主能力（浏览器自动化、宿主钩子）；
//! 2. 把内核事件桥接成 Tauri 事件，前端契约零改动。
//!
//! 内核进程的拉起、JSON-RPC 收发、重连退避都在 `nova-client` 里，
//! 与 TUI / 无头共用同一份实现。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use nova_client::{BrowserHandler, Client, ClientOptions, HookHandler};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

/// 当前活跃的内核客户端；重连时整体替换。
static ACTIVE: OnceLock<RwLock<Option<Arc<Client>>>> = OnceLock::new();

fn active() -> &'static RwLock<Option<Arc<Client>>> {
    ACTIVE.get_or_init(|| RwLock::new(None))
}

pub fn init(app: &AppHandle, cli_path: Option<PathBuf>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(cli_path) = cli_path else {
            warn!("未找到可用的 Nova CLI，内核通道不可用");
            return;
        };

        let mut attempt: u32 = 0;
        loop {
            let client_result = match nova_client::uds::default_daemon_socket_path() {
                Ok(socket_path) => {
                    Client::connect_or_spawn(&cli_path, &socket_path, build_options(&app)).await
                }
                Err(error) => Err(error),
            };
            match client_result {
                Ok(client) => {
                    info!("Nova 内核已连接（WebSocket over UDS 共享守护进程）");
                    attempt = 0;
                    if let Ok(mut slot) = active().write() {
                        *slot = Some(client.clone());
                    }
                    let _ = app.emit("daemon-connected", ());

                    let mut events = client.subscribe();
                    while let Some((name, payload)) = events.recv().await {
                        let _ = app.emit(&name, payload);
                    }

                    warn!("内核事件流结束，准备重启");
                    if let Ok(mut slot) = active().write() {
                        *slot = None;
                    }
                    let _ = app.emit("daemon-disconnected", ());
                }
                Err(error) => {
                    warn!(%error, "拉起内核子进程失败");
                    attempt = attempt.saturating_add(1);
                }
            }

            // 前几次快速重试（冷启动竞态），之后逐步退避。
            let delay = match attempt {
                0 => Duration::from_millis(300),
                n if n < 5 => Duration::from_millis(500 * u64::from(n)),
                _ => Duration::from_secs(5),
            };
            tokio::time::sleep(delay).await;
        }
    });
}

/// 转发一条 IPC 命令到内核并等待结果。
pub async fn invoke(method: &str, params: Value) -> Result<Value, String> {
    let client = {
        let slot = active()
            .read()
            .map_err(|_| "内核通道状态不可用".to_string())?;
        slot.clone()
    }
    .ok_or_else(|| "内核通道未就绪".to_string())?;

    client.invoke(method, params).await
}

/// 桌面退出时先通知内核走优雅关闭（回收所有 PTY 进程树与用户终端），再释放 Client。
pub fn shutdown() {
    let client = active()
        .write()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(client) = client {
        client.shutdown();
    }
}

fn build_options(app: &AppHandle) -> ClientOptions {
    ClientOptions {
        browser: Some(browser_handler(app.clone())),
        hooks: Some(hook_handler()),
    }
}

fn browser_handler(app: AppHandle) -> BrowserHandler {
    Arc::new(move |params: Value| {
        let app = app.clone();
        Box::pin(async move {
            crate::services::browser_sessions::run_command(
                &app,
                params
                    .get("conversationId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty()),
                params.get("action").and_then(Value::as_str).unwrap_or(""),
                params.get("payload").cloned().unwrap_or(Value::Null),
                params.get("timeoutMs").and_then(Value::as_u64),
            )
            .await
        })
    })
}

fn hook_handler() -> HookHandler {
    Arc::new(|operation: String, args: Value| match operation.as_str() {
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
    })
}
