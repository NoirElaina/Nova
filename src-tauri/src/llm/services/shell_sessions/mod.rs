use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tracing::{info, warn};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
const PWSH_PATH: &str = "C:\\Program Files\\PowerShell\\7\\pwsh.exe";

const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const MAX_TIMEOUT_MS: u64 = 1_800_000;
const MARKER_PREFIX: &str = "__NOVA_CMD_END__|";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub background: bool,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSessionStatus {
    pub exists: bool,
    pub alive: bool,
    pub busy: bool,
    pub cwd: Option<String>,
    pub background_pids: Vec<u32>,
    pub background_count: usize,
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug)]
struct StreamEvent {
    stream: StreamKind,
    text: String,
}

#[derive(Debug, Clone)]
struct CommandMarker {
    command_id: String,
    exit_code: i32,
    cwd: String,
    timed_out: bool,
}

struct ShellSession {
    child: Child,
    stdin: ChildStdin,
    events: mpsc::UnboundedReceiver<StreamEvent>,
    last_known_cwd: Option<String>,
    background_pids: HashSet<u32>,
}

struct SessionHandle {
    inner: AsyncMutex<ShellSession>,
}

fn session_registry() -> &'static Mutex<HashMap<String, Arc<SessionHandle>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<SessionHandle>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__default__")
        .to_string()
}

fn normalized_timeout_ms(timeout_ms: Option<u64>) -> u64 {
    timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1_000, MAX_TIMEOUT_MS)
}

fn parse_marker_line(line: &str) -> Option<CommandMarker> {
    let trimmed = line.trim();
    let payload = trimmed.strip_prefix(MARKER_PREFIX)?;
    let mut parts = payload.split('|');
    let command_id = parts.next()?.to_string();
    let exit_code = parts.next()?.parse::<i32>().ok()?;
    let cwd_b64 = parts.next()?;
    let timed_out = parts.next()?.eq_ignore_ascii_case("1");
    let cwd_bytes = BASE64.decode(cwd_b64).ok()?;
    let cwd = String::from_utf8(cwd_bytes).ok()?;
    Some(CommandMarker {
        command_id,
        exit_code,
        cwd,
        timed_out,
    })
}

fn encode_utf8_base64(text: &str) -> String {
    BASE64.encode(text.as_bytes())
}

#[cfg(target_os = "windows")]
fn encode_pwsh_command(command: &str) -> String {
    let mut utf16 = Vec::with_capacity(command.len() * 2);
    for unit in command.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    BASE64.encode(utf16)
}

#[cfg(target_os = "windows")]
fn build_bootstrap_script() -> String {
    [
        "[Console]::InputEncoding = [System.Text.Encoding]::UTF8",
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8",
        "$OutputEncoding = [System.Text.Encoding]::UTF8",
        "$ProgressPreference = 'SilentlyContinue'",
        "$ErrorActionPreference = 'Continue'",
        "$env:NO_COLOR = '1'",
        "if ($PSStyle) { $PSStyle.OutputRendering = 'PlainText' }",
        "function global:prompt { '' }",
        "",
    ]
    .join("\n")
}

#[cfg(target_os = "windows")]
fn build_foreground_wrapper(command_id: &str, command: &str) -> String {
    let encoded = encode_utf8_base64(command);
    format!(
        r#"$__novaCommandId = '{command_id}'
$__novaEncodedCommand = '{encoded}'
$__novaCommand = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($__novaEncodedCommand))
$env:NO_COLOR = '1'
if ($PSStyle) {{ $PSStyle.OutputRendering = 'PlainText' }}
$global:LASTEXITCODE = 0
try {{
    Invoke-Expression $__novaCommand
    $__novaExitCode = if ($LASTEXITCODE -is [int]) {{ [int]$LASTEXITCODE }} else {{ 0 }}
}} catch {{
    $__novaExitCode = 1
    Write-Error $_
}}
$__novaCwd = (Get-Location).Path
$__novaCwdB64 = [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($__novaCwd))
$__novaMarker = "{prefix}$__novaCommandId|$__novaExitCode|$__novaCwdB64|0"
Write-Output $__novaMarker
[Console]::Error.WriteLine($__novaMarker)
"#,
        prefix = MARKER_PREFIX
    )
}

#[cfg(target_os = "windows")]
fn build_background_wrapper(command_id: &str, command: &str) -> String {
    let encoded = encode_pwsh_command(command);
    format!(
        r#"$__novaCommandId = '{command_id}'
$__novaCwd = (Get-Location).Path
$__nova = Start-Process -FilePath '{pwsh}' -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand','{encoded}') -WorkingDirectory $__novaCwd -WindowStyle Hidden -RedirectStandardOutput 'NUL' -RedirectStandardError 'NUL' -PassThru
[pscustomobject]@{{
    ok = $true
    background = $true
    pid = $__nova.Id
    cwd = $__novaCwd
}} | ConvertTo-Json -Compress
"#,
        pwsh = PWSH_PATH,
    )
}

#[cfg(not(target_os = "windows"))]
fn build_bootstrap_script() -> String {
    String::new()
}

#[cfg(not(target_os = "windows"))]
fn build_foreground_wrapper(command_id: &str, command: &str) -> String {
    let encoded = encode_utf8_base64(command);
    format!(
        "NOVA_CMD_ID='{command_id}'\nNOVA_CMD=$(printf '%s' '{encoded}' | base64 -d)\neval \"$NOVA_CMD\"\nNOVA_EXIT=$?\nNOVA_CWD_B64=$(pwd | base64 | tr -d '\\n')\nNOVA_MARKER='{prefix}'\"$NOVA_CMD_ID|$NOVA_EXIT|$NOVA_CWD_B64|0\"\nprintf '%s\\n' \"$NOVA_MARKER\"\nprintf '%s\\n' \"$NOVA_MARKER\" >&2\n",
        prefix = MARKER_PREFIX
    )
}

#[cfg(not(target_os = "windows"))]
fn build_background_wrapper(command_id: &str, command: &str) -> String {
    let escaped = command.replace('\'', "'\"'\"'");
    let _ = command_id;
    format!(
        "sh -lc '{}' >/dev/null 2>&1 &\nprintf '{{\"ok\":true,\"background\":true,\"pid\":%s,\"cwd\":\"%s\"}}\\n' \"$!\" \"$PWD\"\n",
        escaped
    )
}

fn spawn_stream_reader<R>(
    stream: R,
    stream_kind: StreamKind,
    tx: mpsc::UnboundedSender<StreamEvent>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let _ = tx.send(StreamEvent {
                        stream: stream_kind,
                        text: line,
                    });
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn make_shell_command() -> Command {
    let mut command = Command::new(PWSH_PATH);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-NoExit",
            "-Command",
            "-",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(not(target_os = "windows"))]
fn make_shell_command() -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

async fn spawn_session(initial_cwd: Option<&str>) -> Result<ShellSession, String> {
    let mut child = make_shell_command()
        .spawn()
        .map_err(|error| format!("Failed to start shell session: {}", error))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Shell session missing stdin pipe".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Shell session missing stdout pipe".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Shell session missing stderr pipe".to_string())?;

    let (tx, rx) = mpsc::unbounded_channel();
    spawn_stream_reader(stdout, StreamKind::Stdout, tx.clone());
    spawn_stream_reader(stderr, StreamKind::Stderr, tx);

    let mut session = ShellSession {
        child,
        stdin,
        events: rx,
        last_known_cwd: None,
        background_pids: HashSet::new(),
    };

    let mut bootstrap = build_bootstrap_script();
    if let Some(cwd) = initial_cwd.filter(|value| !value.trim().is_empty()) {
        #[cfg(target_os = "windows")]
        {
            bootstrap.push_str(&format!(
                "$__novaRestoreCwd = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{}'))\nSet-Location -LiteralPath $__novaRestoreCwd -ErrorAction SilentlyContinue\n",
                encode_utf8_base64(cwd)
            ));
        }
        #[cfg(not(target_os = "windows"))]
        {
            bootstrap.push_str(&format!(
                "cd '{}' 2>/dev/null || true\n",
                cwd.replace('\'', "'\"'\"'")
            ));
        }
    }

    if !bootstrap.is_empty() {
        session
            .stdin
            .write_all(bootstrap.as_bytes())
            .await
            .map_err(|error| format!("Failed to bootstrap shell session: {}", error))?;
        session
            .stdin
            .flush()
            .await
            .map_err(|error| format!("Failed to flush shell bootstrap: {}", error))?;
    }

    session.last_known_cwd = initial_cwd.map(|value| value.to_string()).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
    });

    Ok(session)
}

async fn ensure_session_alive(session: &mut ShellSession) -> Result<(), String> {
    match session.child.try_wait() {
        Ok(Some(status)) => {
            warn!(status = %status, "shell session exited unexpectedly; recreating");
            let cwd = session.last_known_cwd.clone();
            *session = spawn_session(cwd.as_deref()).await?;
        }
        Ok(None) => {}
        Err(error) => {
            warn!(error = %error, "failed to probe shell session status; recreating");
            let cwd = session.last_known_cwd.clone();
            *session = spawn_session(cwd.as_deref()).await?;
        }
    }
    Ok(())
}

async fn restart_session(
    session: &mut ShellSession,
    cwd_override: Option<&str>,
) -> Result<(), String> {
    let cwd = cwd_override
        .map(str::to_string)
        .or_else(|| session.last_known_cwd.clone());
    let background_pids = std::mem::take(&mut session.background_pids);
    for pid in background_pids {
        kill_pid(pid);
    }
    let _ = session.child.kill().await;
    *session = spawn_session(cwd.as_deref()).await?;
    Ok(())
}

fn trim_trailing_newline(text: String) -> String {
    text.trim_end_matches(['\r', '\n']).to_string()
}

async fn execute_wrapped_command(
    session: &mut ShellSession,
    conversation_id: Option<&str>,
    script: &str,
    timeout_ms: u64,
) -> Result<ShellExecutionResult, String> {
    ensure_session_alive(session).await?;

    let timeout_at = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let command_id = uuid::Uuid::new_v4().to_string();
    let wrapped = script.replace("{command_id}", &command_id);

    session
        .stdin
        .write_all(wrapped.as_bytes())
        .await
        .map_err(|error| format!("Failed to write shell command: {}", error))?;
    session
        .stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("Failed to finish shell command write: {}", error))?;
    session
        .stdin
        .flush()
        .await
        .map_err(|error| format!("Failed to flush shell command: {}", error))?;

    let cwd_before = session.last_known_cwd.clone();
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut resolved_marker: Option<CommandMarker> = None;

    loop {
        if stdout_done && stderr_done {
            break;
        }

        if crate::llm::cancellation::is_cancelled(conversation_id) {
            warn!("shell command cancelled; restarting session");
            restart_session(session, None).await?;
            session.last_known_cwd = cwd_before;
            return Ok(ShellExecutionResult {
                stdout: trim_trailing_newline(stdout),
                stderr: trim_trailing_newline(stderr),
                exit_code: None,
                cwd: session.last_known_cwd.clone(),
                timed_out: false,
                cancelled: true,
                background: false,
                pid: None,
            });
        }

        let now = tokio::time::Instant::now();
        if now >= timeout_at {
            warn!("shell command timed out; restarting session");
            restart_session(session, None).await?;
            session.last_known_cwd = cwd_before;
            return Ok(ShellExecutionResult {
                stdout: trim_trailing_newline(stdout),
                stderr: trim_trailing_newline(stderr),
                exit_code: None,
                cwd: session.last_known_cwd.clone(),
                timed_out: true,
                cancelled: false,
                background: false,
                pid: None,
            });
        }

        let remaining = timeout_at
            .saturating_duration_since(now)
            .min(Duration::from_millis(100));
        let maybe_event = tokio::time::timeout(remaining, session.events.recv()).await;
        let event = match maybe_event {
            Ok(Some(event)) => event,
            Ok(None) => {
                warn!("shell session stream closed unexpectedly; restarting");
                restart_session(session, None).await?;
                session.last_known_cwd = cwd_before;
                return Err("Shell session closed unexpectedly".to_string());
            }
            Err(_) => continue,
        };

        if let Some(marker) = parse_marker_line(&event.text) {
            if marker.command_id == command_id {
                session.last_known_cwd = Some(marker.cwd.clone());
                resolved_marker = Some(marker);
                match event.stream {
                    StreamKind::Stdout => stdout_done = true,
                    StreamKind::Stderr => stderr_done = true,
                }
                continue;
            }
        }

        match event.stream {
            StreamKind::Stdout => stdout.push_str(&event.text),
            StreamKind::Stderr => stderr.push_str(&event.text),
        }
    }

    let marker =
        resolved_marker.ok_or_else(|| "Shell command finished without marker".to_string())?;
    Ok(ShellExecutionResult {
        stdout: trim_trailing_newline(stdout),
        stderr: trim_trailing_newline(stderr),
        exit_code: Some(marker.exit_code),
        cwd: Some(marker.cwd),
        timed_out: marker.timed_out,
        cancelled: false,
        background: false,
        pid: None,
    })
}

async fn get_or_create_handle(
    conversation_id: Option<&str>,
    initial_cwd: Option<&str>,
) -> Result<Arc<SessionHandle>, String> {
    let key = scope_key(conversation_id);
    if let Some(existing) = session_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .cloned()
    {
        return Ok(existing);
    }

    let session = spawn_session(initial_cwd).await?;
    let handle = Arc::new(SessionHandle {
        inner: AsyncMutex::new(session),
    });

    let mut registry = session_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    Ok(registry
        .entry(key)
        .or_insert_with(|| handle.clone())
        .clone())
}

fn kill_pid(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn background_result_json(pid: u32, cwd: &str) -> String {
    serde_json::json!({
        "ok": true,
        "background": true,
        "pid": pid,
        "cwd": cwd,
    })
    .to_string()
}

pub async fn run_foreground(
    conversation_id: Option<&str>,
    command: &str,
    timeout_ms: Option<u64>,
    initial_cwd: Option<&str>,
) -> Result<ShellExecutionResult, String> {
    let handle = get_or_create_handle(conversation_id, initial_cwd).await?;
    let mut session = handle.inner.lock().await;
    let command_id = "{command_id}";
    let script = build_foreground_wrapper(command_id, command);
    execute_wrapped_command(
        &mut session,
        conversation_id,
        &script,
        normalized_timeout_ms(timeout_ms),
    )
    .await
}

pub async fn run_background(
    conversation_id: Option<&str>,
    command: &str,
    initial_cwd: Option<&str>,
) -> Result<ShellExecutionResult, String> {
    let handle = get_or_create_handle(conversation_id, initial_cwd).await?;
    let mut session = handle.inner.lock().await;
    let command_id = "{command_id}";
    let script = build_background_wrapper(command_id, command);
    let mut result = execute_wrapped_command(
        &mut session,
        conversation_id,
        &script,
        normalized_timeout_ms(Some(30_000)),
    )
    .await?;

    let payload: serde_json::Value = serde_json::from_str(result.stdout.trim())
        .map_err(|error| format!("Invalid background shell response: {}", error))?;
    let pid = payload
        .get("pid")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "Background shell response missing pid".to_string())?;
    let cwd = payload
        .get("cwd")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if !cwd.trim().is_empty() {
        session.last_known_cwd = Some(cwd.clone());
    }
    session.background_pids.insert(pid);
    result.stdout = background_result_json(pid, &cwd);
    result.background = true;
    result.pid = Some(pid);
    Ok(result)
}

pub async fn reset_session(
    conversation_id: Option<&str>,
    workspace_root: Option<&str>,
) -> Result<(), String> {
    let handle = get_or_create_handle(conversation_id, workspace_root).await?;
    let mut session = handle.inner.lock().await;
    restart_session(&mut session, workspace_root).await
}

pub async fn session_status(conversation_id: Option<&str>) -> ShellSessionStatus {
    let key = scope_key(conversation_id);
    let handle = session_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .cloned();

    let Some(handle) = handle else {
        return ShellSessionStatus {
            exists: false,
            alive: false,
            busy: false,
            cwd: None,
            background_pids: Vec::new(),
            background_count: 0,
        };
    };

    let Ok(mut session) = handle.inner.try_lock() else {
        return ShellSessionStatus {
            exists: true,
            alive: true,
            busy: true,
            cwd: None,
            background_pids: Vec::new(),
            background_count: 0,
        };
    };

    let alive = matches!(session.child.try_wait(), Ok(None));
    let mut background_pids: Vec<u32> = session.background_pids.iter().copied().collect();
    background_pids.sort_unstable();

    ShellSessionStatus {
        exists: true,
        alive,
        busy: false,
        cwd: session.last_known_cwd.clone(),
        background_count: background_pids.len(),
        background_pids,
    }
}

pub async fn close_session(conversation_id: Option<&str>) {
    let key = scope_key(conversation_id);
    let handle = session_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&key);
    if let Some(handle) = handle {
        let mut session = handle.inner.lock().await;
        let pids = std::mem::take(&mut session.background_pids);
        for pid in pids {
            kill_pid(pid);
        }
        let _ = session.child.kill().await;
        info!(conversation_scope = %key, "shell session closed");
    }
}

pub async fn close_all_sessions() {
    let handles: Vec<(String, Arc<SessionHandle>)> = {
        let mut registry = session_registry()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        registry.drain().collect()
    };

    for (key, handle) in handles {
        let mut session = handle.inner.lock().await;
        let pids = std::mem::take(&mut session.background_pids);
        for pid in pids {
            kill_pid(pid);
        }
        let _ = session.child.kill().await;
        info!(conversation_scope = %key, "shell session closed during global cleanup");
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_marker_line, session_status, BASE64};
    use base64::Engine as _;

    #[test]
    fn parses_marker_line() {
        let cwd = "D:\\work\\repo";
        let encoded = BASE64.encode(cwd.as_bytes());
        let line = format!("__NOVA_CMD_END__|cmd-1|7|{}|0", encoded);
        let marker = parse_marker_line(&line).expect("marker");
        assert_eq!(marker.command_id, "cmd-1");
        assert_eq!(marker.exit_code, 7);
        assert_eq!(marker.cwd, cwd);
        assert!(!marker.timed_out);
    }

    #[tokio::test]
    async fn status_for_missing_session_does_not_create_one() {
        let status = session_status(Some("__nova_missing_shell_status_test__")).await;
        assert!(!status.exists);
        assert!(!status.alive);
        assert!(!status.busy);
        assert!(status.cwd.is_none());
        assert_eq!(status.background_count, 0);
        assert!(status.background_pids.is_empty());
    }
}
