use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

const DEFAULT_SCOPE: &str = "__default__";
const BROWSER_COMMAND_EVENT: &str = "nova-browser-command";
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const SNAPSHOT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 60_000;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<BrowserSessionState>> = OnceLock::new();

#[derive(Debug, Clone)]
struct BrowserSession {
    label: String,
    current_url: Option<String>,
    updated_at_ms: u128,
}

#[derive(Default)]
struct BrowserSessionState {
    sessions: HashMap<String, BrowserSession>,
    pending: HashMap<String, oneshot::Sender<Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationCommand {
    pub conversation_id: String,
    pub request_id: String,
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationResult {
    pub request_id: String,
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
}

fn state() -> &'static Mutex<BrowserSessionState> {
    STATE.get_or_init(|| Mutex::new(BrowserSessionState::default()))
}

fn scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_SCOPE)
        .to_string()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn next_request_id() -> String {
    let count = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("browser-{}-{count}", now_ms())
}

pub fn register_session(
    conversation_id: Option<&str>,
    label: String,
    current_url: Option<String>,
) -> Result<(), String> {
    let key = scope_key(conversation_id);
    let mut guard = state()
        .lock()
        .map_err(|_| "browser session registry poisoned".to_string())?;
    guard.sessions.insert(
        key,
        BrowserSession {
            label,
            current_url,
            updated_at_ms: now_ms(),
        },
    );
    Ok(())
}

pub fn unregister_session(conversation_id: Option<&str>, label: &str) -> Result<(), String> {
    let key = scope_key(conversation_id);
    let mut guard = state()
        .lock()
        .map_err(|_| "browser session registry poisoned".to_string())?;

    let should_remove = guard
        .sessions
        .get(&key)
        .map(|session| session.label == label)
        .unwrap_or(false);
    if should_remove {
        guard.sessions.remove(&key);
    }
    Ok(())
}

pub fn update_session_url(
    conversation_id: Option<&str>,
    label: &str,
    current_url: Option<String>,
) -> Result<(), String> {
    let key = scope_key(conversation_id);
    let mut guard = state()
        .lock()
        .map_err(|_| "browser session registry poisoned".to_string())?;

    if let Some(session) = guard.sessions.get_mut(&key) {
        if session.label == label {
            session.current_url = current_url;
            session.updated_at_ms = now_ms();
        }
    }
    Ok(())
}

pub fn complete_command(result: BrowserAutomationResult) -> Result<(), String> {
    let sender = {
        let mut guard = state()
            .lock()
            .map_err(|_| "browser session registry poisoned".to_string())?;
        guard.pending.remove(&result.request_id)
    };

    let Some(sender) = sender else {
        return Ok(());
    };

    let payload = if result.ok {
        json!({
            "ok": true,
            "result": result.result.unwrap_or(Value::Null),
        })
    } else {
        json!({
            "ok": false,
            "error": result.error.unwrap_or_else(|| "browser command failed".to_string()),
            "result": result.result.unwrap_or(Value::Null),
        })
    };

    let _ = sender.send(payload);
    Ok(())
}

pub async fn run_command(
    app: &AppHandle,
    conversation_id: Option<&str>,
    action: &str,
    payload: Value,
    timeout_ms: Option<u64>,
) -> Value {
    let key = scope_key(conversation_id);
    let session = {
        let guard = match state().lock() {
            Ok(guard) => guard,
            Err(_) => {
                return json!({
                    "ok": false,
                    "error": "browser session registry poisoned",
                });
            }
        };
        guard.sessions.get(&key).cloned()
    };

    let Some(session) = session else {
        return json!({
            "ok": true,
            "available": false,
            "message": "Nova Browser tab is not open for this conversation.",
            "hint": "Open the workspace Browser tab first, then retry.",
        });
    };

    let request_id = next_request_id();
    let (sender, receiver) = oneshot::channel();
    {
        let mut guard = match state().lock() {
            Ok(guard) => guard,
            Err(_) => {
                return json!({
                    "ok": false,
                    "error": "browser session registry poisoned",
                });
            }
        };
        guard.pending.insert(request_id.clone(), sender);
    }

    let event = BrowserAutomationCommand {
        conversation_id: key,
        request_id: request_id.clone(),
        action: action.to_string(),
        payload: json!({
            "label": session.label,
            "currentUrl": session.current_url,
            "updatedAtMs": session.updated_at_ms,
            "input": payload,
        }),
    };

    if let Err(error) = app.emit(BROWSER_COMMAND_EVENT, event) {
        if let Ok(mut guard) = state().lock() {
            guard.pending.remove(&request_id);
        }
        return json!({
            "ok": false,
            "error": format!("failed to dispatch browser command: {error}"),
        });
    }

    let default_timeout_ms = if action == "snapshot" {
        SNAPSHOT_TIMEOUT_MS
    } else {
        DEFAULT_TIMEOUT_MS
    };
    let timeout_ms = timeout_ms
        .unwrap_or(default_timeout_ms)
        .clamp(1_000, MAX_TIMEOUT_MS);
    match tokio::time::timeout(Duration::from_millis(timeout_ms), receiver).await {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => json!({
            "ok": false,
            "error": "browser command receiver closed",
        }),
        Err(_) => {
            if let Ok(mut guard) = state().lock() {
                guard.pending.remove(&request_id);
            }
            json!({
                "ok": false,
                "error": format!("browser command timed out after {timeout_ms}ms"),
            })
        }
    }
}
