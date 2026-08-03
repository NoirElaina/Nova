use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use regex::Regex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
const PWSH_PATH: &str = "C:\\Program Files\\PowerShell\\7\\pwsh.exe";

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 1_800_000;
const MARKER_PREFIX: &str = "__NOVA_CMD_END__|";
const READY_MARKER: &str = "__NOVA_SHELL_READY__";

// 卡顿看门狗:输出长时间不增长且末尾命中交互式 prompt 正则时,
// 判定命令在等待 stdin,提前终止并提示模型改用管道喂输入。
const STALL_THRESHOLD_MS: u64 = 45_000;
const STALL_TAIL_BYTES: usize = 1024;
// 双流 marker 不必死等：管道全缓冲时另一路可能迟迟不刷出。
// 任一端收到本命令 marker 后，最多再等这么久收另一端，然后结束。
const MARKER_PEER_GRACE_MS: u64 = 80;
const BOOTSTRAP_READY_TIMEOUT_MS: u64 = 15_000;

fn prompt_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"(?i)\(y/n\)").unwrap(),
            Regex::new(r"(?i)\[y/n\]").unwrap(),
            Regex::new(r"(?i)\(yes/no\)").unwrap(),
            Regex::new(r"(?i)\b(?:Do you|Would you|Shall I|Are you sure|Ready to)\b.*\?\s*$").unwrap(),
            Regex::new(r"(?i)Press (any key|Enter)").unwrap(),
            Regex::new(r"(?i)Continue\?").unwrap(),
            Regex::new(r"(?i)Overwrite\?").unwrap(),
        ]
    })
}

fn looks_like_prompt(text: &str) -> bool {
    let last_line = text.trim_end().split('\n').last().unwrap_or("");
    prompt_patterns().iter().any(|p| p.is_match(last_line))
}

fn extract_tail(stdout: &str, stderr: &str, max_bytes: usize) -> String {
    let combined = format!("{}\n{}", stdout, stderr);
    let bytes = combined.as_bytes();
    if bytes.len() <= max_bytes {
        return combined;
    }
    let mut start = bytes.len() - max_bytes;
    while start < bytes.len() && !combined.is_char_boundary(start) {
        start += 1;
    }
    combined[start..].to_string()
}

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
    let cwd = crate::command::workspace::display_path_text(&String::from_utf8(cwd_bytes).ok()?);
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

fn display_cwd_opt(value: Option<String>) -> Option<String> {
    value.map(|text| crate::command::workspace::display_path_text(&text))
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
fn build_bootstrap_init_script() -> String {
    r#"[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'Continue'
$env:NO_COLOR = '1'
if ($PSStyle) { $PSStyle.OutputRendering = 'PlainText' }
function global:prompt { '' }
"#
    .to_string()
}

#[cfg(target_os = "windows")]
fn build_ready_marker_script() -> String {
    // 直写 Console 并 Flush，避免管道全缓冲导致 ready 迟迟不到
    format!(
        r#"[Console]::Out.WriteLine('{ready}')
[Console]::Out.Flush()
[Console]::Error.WriteLine('{ready}')
[Console]::Error.Flush()
"#,
        ready = READY_MARKER
    )
}

#[cfg(target_os = "windows")]
fn build_foreground_wrapper(command_id: &str, command: &str) -> String {
    let encoded = encode_utf8_base64(command);
    // 不用 Write-Output 发 marker：成功流在重定向时可能块缓冲，双 marker 死等会拖到 timeout。
    format!(
        r#"$__novaCommandId = '{command_id}'
$__novaEncodedCommand = '{encoded}'
$__novaCommand = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($__novaEncodedCommand))
$env:NO_COLOR = '1'
if ($PSStyle) {{ $PSStyle.OutputRendering = 'PlainText' }}
$global:LASTEXITCODE = 0
try {{
    Invoke-Expression $__novaCommand
    $__novaCommandSucceeded = $?
    $__novaExitCode = if ($LASTEXITCODE -is [int]) {{
        [int]$LASTEXITCODE
    }} elseif ($__novaCommandSucceeded) {{
        0
    }} else {{
        1
    }}
}} catch {{
    $__novaExitCode = 1
    Write-Error $_
}}
# 先刷出命令输出，再写 marker，避免管道块缓冲把 stdout 和 marker 一起卡住
[Console]::Out.Flush()
[Console]::Error.Flush()
$__novaCwd = (Get-Location).Path
$__novaCwdB64 = [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($__novaCwd))
$__novaMarker = "{prefix}$__novaCommandId|$__novaExitCode|$__novaCwdB64|0"
[Console]::Out.WriteLine($__novaMarker)
[Console]::Out.Flush()
[Console]::Error.WriteLine($__novaMarker)
[Console]::Error.Flush()
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
$__novaCwdB64 = [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($__novaCwd))
$__novaMarker = "{prefix}$__novaCommandId|0|$__novaCwdB64|0"
[Console]::Out.WriteLine($__novaMarker)
[Console]::Out.Flush()
[Console]::Error.WriteLine($__novaMarker)
[Console]::Error.Flush()
"#,
        pwsh = PWSH_PATH,
        prefix = MARKER_PREFIX,
        command_id = command_id,
        encoded = encoded,
    )
}

#[cfg(not(target_os = "windows"))]
fn build_bootstrap_init_script() -> String {
    String::new()
}

#[cfg(not(target_os = "windows"))]
fn build_ready_marker_script() -> String {
    // ready 写到 stdout+stderr，spawn 侧等到任一端即可认为 shell 已可接收命令
    format!(
        "printf '%s\\n' '{ready}'\nprintf '%s\\n' '{ready}' >&2\n",
        ready = READY_MARKER
    )
}

#[cfg(not(target_os = "windows"))]
fn build_foreground_wrapper(command_id: &str, command: &str) -> String {
    let encoded = encode_utf8_base64(command);
    // 优先 stdbuf 行缓冲；没有则退回普通 printf。结束时再尝试 fflush 不可用，
    // 依赖 execute 侧「单 marker + grace」避免双流死等。
    format!(
        "NOVA_CMD_ID='{command_id}'\nNOVA_CMD=$(printf '%s' '{encoded}' | base64 -d 2>/dev/null || printf '%s' '{encoded}' | base64 -D)\neval \"$NOVA_CMD\"\nNOVA_EXIT=$?\nNOVA_CWD_B64=$(pwd | base64 | tr -d '\\n')\nNOVA_MARKER='{prefix}'\"$NOVA_CMD_ID|$NOVA_EXIT|$NOVA_CWD_B64|0\"\nprintf '%s\\n' \"$NOVA_MARKER\"\nprintf '%s\\n' \"$NOVA_MARKER\" >&2\n",
        prefix = MARKER_PREFIX
    )
}

#[cfg(not(target_os = "windows"))]
fn build_background_wrapper(command_id: &str, command: &str) -> String {
    let escaped = command.replace('\'', "'\"'\"'");
    // 后台命令必须输出 marker，否则 execute_wrapped_command 会等到 timeout。
    // marker 的 exit_code 固定为 0（后台进程已成功启动），cwd 为当前目录的 base64。
    //
    // 使用 setsid 启动后台进程，使其成为新会话 leader + 新进程组 leader。
    // 这样后台进程的 pid == pgid，kill_pid 可以用 libc::kill(-pid, SIGTERM)
    // 杀掉整个进程组（含所有子进程），避免孤儿进程。
    // setsid 是 util-linux/coreutils 标准命令，几乎所有 Unix 系统自带。
    format!(
        "setsid sh -lc '{}' >/dev/null 2>&1 &\nNOVA_BG_PID=$!\nNOVA_BG_CWD_B64=$(pwd | base64 | tr -d '\\n')\nprintf '{{\"ok\":true,\"background\":true,\"pid\":%s,\"cwd\":\"%s\"}}\\n' \"$NOVA_BG_PID\" \"$PWD\"\nprintf '{prefix}{command_id}|0|%s|0\\n' \"$NOVA_BG_CWD_B64\"\nprintf '{prefix}{command_id}|0|%s|0\\n' \"$NOVA_BG_CWD_B64\" >&2\n",
        escaped,
        prefix = MARKER_PREFIX,
        command_id = command_id,
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
        .args(["-NoProfile", "-NoLogo", "-NonInteractive", "-NoExit", "-Command", "-"])
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

    // 顺序：init → cwd → ready。ready 必须最后，表示会话已可接收用户命令。
    let mut bootstrap = build_bootstrap_init_script();
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
    bootstrap.push_str(&build_ready_marker_script());

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
    wait_for_ready_marker(&mut session).await?;

    session.last_known_cwd = initial_cwd
        .map(crate::command::workspace::display_path_text)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| crate::command::workspace::display_path_string(&path))
        });

    Ok(session)
}

fn is_ready_marker_line(line: &str) -> bool {
    line.trim() == READY_MARKER
}

/// 等到 bootstrap ready（stdout 或 stderr 任一端）。超时则失败，由上层重建会话。
async fn wait_for_ready_marker(session: &mut ShellSession) -> Result<(), String> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(BOOTSTRAP_READY_TIMEOUT_MS);
    let mut saw_ready = false;
    // 双端都可能发 ready；收到第一个即可，再短等另一端以免残留进下一命令。
    let mut peer_deadline: Option<tokio::time::Instant> = None;
    let mut ready_count = 0_u8;

    while ready_count < 2 {
        if saw_ready {
            if let Some(peer_at) = peer_deadline {
                if tokio::time::Instant::now() >= peer_at {
                    break;
                }
            }
        }

        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err("Shell session bootstrap timed out waiting for ready marker".into());
        }

        let slice = if let Some(peer_at) = peer_deadline {
            peer_at.saturating_duration_since(now).min(deadline.saturating_duration_since(now))
        } else {
            deadline.saturating_duration_since(now)
        };

        match tokio::time::timeout(slice, session.events.recv()).await {
            Ok(Some(event)) => {
                if is_ready_marker_line(&event.text) {
                    saw_ready = true;
                    ready_count = ready_count.saturating_add(1);
                    if peer_deadline.is_none() {
                        peer_deadline = Some(
                            tokio::time::Instant::now()
                                + Duration::from_millis(MARKER_PEER_GRACE_MS),
                        );
                    }
                }
                // bootstrap 期间其它输出直接丢弃
            }
            Ok(None) => {
                return Err("Shell session closed during bootstrap".into());
            }
            Err(_) => {
                if saw_ready {
                    break;
                }
            }
        }
    }

    if !saw_ready {
        return Err("Shell session bootstrap failed without ready marker".into());
    }
    Ok(())
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

async fn kill_session_tree(session: &mut ShellSession) {
    #[cfg(target_os = "windows")]
    if let Some(pid) = session.child.id() {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status();
    }
    let _ = session.child.kill().await;
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
    kill_session_tree(session).await;
    *session = spawn_session(cwd.as_deref()).await?;
    Ok(())
}

fn trim_trailing_newline(text: String) -> String {
    text.trim_end_matches(['\r', '\n']).to_string()
}

async fn execute_wrapped_command(
    session: &mut ShellSession,
    script: &str,
    timeout_ms: u64,
    cancel_token: CancellationToken,
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
    let mut first_marker_at: Option<tokio::time::Instant> = None;
    let mut last_output_growth_at = tokio::time::Instant::now();

    loop {
        let both_markers = stdout_done && stderr_done;
        let grace_elapsed = first_marker_at.is_some_and(|at| {
            tokio::time::Instant::now().duration_since(at)
                >= Duration::from_millis(MARKER_PEER_GRACE_MS)
        });
        // 双 marker 齐了，或已收到一端 marker 且宽限期过：立即结束，避免管道缓冲死等。
        if both_markers || (resolved_marker.is_some() && grace_elapsed) {
            break;
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
                cwd: display_cwd_opt(session.last_known_cwd.clone()),
                timed_out: true,
                cancelled: false,
                background: false,
                pid: None,
            });
        }

        // 卡顿看门狗:输出长时间不增长且末尾命中交互式 prompt 正则时,
        // 判定命令在等待 stdin,提前终止并提示模型改用管道喂输入。
        if resolved_marker.is_none()
            && now.duration_since(last_output_growth_at) >= Duration::from_millis(STALL_THRESHOLD_MS)
        {
            let tail = extract_tail(&stdout, &stderr, STALL_TAIL_BYTES);
            if looks_like_prompt(&tail) {
                warn!("shell command appears blocked on interactive input; restarting");
                restart_session(session, None).await?;
                session.last_known_cwd = cwd_before;
                let notice = "命令疑似在等待交互输入,已终止。请用 piped input 重试(如 `echo y | command`),或加非交互标志。";
                return Ok(ShellExecutionResult {
                    stdout: trim_trailing_newline(stdout),
                    stderr: format!("{}\n{}", notice, trim_trailing_newline(stderr)),
                    exit_code: None,
                    cwd: display_cwd_opt(session.last_known_cwd.clone()),
                    timed_out: true,
                    cancelled: false,
                    background: false,
                    pid: None,
                });
            }
            // 末尾不像 prompt,重置计时器,继续等慢命令。
            last_output_growth_at = now;
        }

        let remaining = timeout_at.saturating_duration_since(now);
        let wait_budget = if let Some(at) = first_marker_at {
            let grace_left = Duration::from_millis(MARKER_PEER_GRACE_MS)
                .saturating_sub(now.duration_since(at));
            remaining.min(grace_left.max(Duration::from_millis(1)))
        } else {
            remaining
        };

        let maybe_event = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                warn!("shell command cancelled; restarting session");
                restart_session(session, None).await?;
                session.last_known_cwd = cwd_before;
                return Ok(ShellExecutionResult {
                    stdout: trim_trailing_newline(stdout),
                    stderr: trim_trailing_newline(stderr),
                    exit_code: None,
                    cwd: display_cwd_opt(session.last_known_cwd.clone()),
                    timed_out: false,
                    cancelled: true,
                    background: false,
                    pid: None,
                });
            }
            result = tokio::time::timeout(wait_budget, session.events.recv()) => result,
        };

        let event = match maybe_event {
            Ok(Some(event)) => event,
            Ok(None) => {
                warn!("shell session stream closed unexpectedly; restarting");
                restart_session(session, None).await?;
                session.last_known_cwd = cwd_before;
                return Err("Shell session closed unexpectedly".to_string());
            }
            Err(_) => {
                // 宽限期到：若已有 marker 则结束；否则继续等命令本身
                if resolved_marker.is_some() {
                    break;
                }
                continue;
            }
        };

        if is_ready_marker_line(&event.text) {
            // 会话重建后的残留 ready，不进输出
            continue;
        }

        if let Some(marker) = parse_marker_line(&event.text) {
            if marker.command_id == command_id {
                session.last_known_cwd = Some(marker.cwd.clone());
                resolved_marker = Some(marker);
                if first_marker_at.is_none() {
                    first_marker_at = Some(tokio::time::Instant::now());
                }
                match event.stream {
                    StreamKind::Stdout => stdout_done = true,
                    StreamKind::Stderr => stderr_done = true,
                }
                continue;
            }
            // 其它命令的残留 marker：丢弃，绝不写进 stdout/stderr
            continue;
        }

        match event.stream {
            StreamKind::Stdout => {
                if !stdout_done {
                    stdout.push_str(&event.text);
                    last_output_growth_at = tokio::time::Instant::now();
                }
            }
            StreamKind::Stderr => {
                if !stderr_done {
                    stderr.push_str(&event.text);
                    last_output_growth_at = tokio::time::Instant::now();
                }
            }
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
        // Windows: taskkill /T /F 杀整个进程树（Windows 内核跟踪的进程树）
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status();
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Unix: 后台进程通过 setsid 启动，pid == pgid（进程组 leader）。
        // kill(-pgid, SIGTERM) 发信号给整个进程组，杀掉后台进程及其所有子进程，
        // 避免子进程变孤儿（父进程被杀后 reparent 到 init）。
        // 用 libc 直接系统调用，不 fork kill 进程，更可靠。
        let pgid = pid as i32;
        unsafe {
            // 先 SIGTERM 给优雅退出的机会
            let _ = libc::kill(-pgid, libc::SIGTERM);
        }
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
    let cancel_token = crate::llm::cancellation::get_token(conversation_id);
    let handle = get_or_create_handle(conversation_id, initial_cwd).await?;
    let mut session = handle.inner.lock().await;
    let command_id = "{command_id}";
    let script = build_foreground_wrapper(command_id, command);
    execute_wrapped_command(&mut session, &script, normalized_timeout_ms(timeout_ms), cancel_token)
        .await
}

pub async fn run_background(
    conversation_id: Option<&str>,
    command: &str,
    initial_cwd: Option<&str>,
) -> Result<ShellExecutionResult, String> {
    let cancel_token = crate::llm::cancellation::get_token(conversation_id);
    let handle = get_or_create_handle(conversation_id, initial_cwd).await?;
    let mut session = handle.inner.lock().await;
    let command_id = "{command_id}";
    let script = build_background_wrapper(command_id, command);
    let mut result =
        execute_wrapped_command(&mut session, &script, normalized_timeout_ms(Some(30_000)), cancel_token)
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
    let cwd = crate::command::workspace::display_path_text(&cwd);
    if !cwd.trim().is_empty() {
        session.last_known_cwd = Some(cwd.clone());
    }
    session.background_pids.insert(pid);
    result.stdout = background_result_json(pid, &cwd);
    result.background = true;
    result.pid = Some(pid);
    // 后台命令成功启动后，marker 已正常返回（exit_code=0），不应再标记为 timeout。
    // 此前由于 background wrapper 未输出 marker 导致 30s 超时，result.timed_out 被置为 true，
    // BashTool 据此返回 "command timed out" 错误。现在 wrapper 已补 marker，但仍需在此
    // 显式重置，防止 marker 解析路径与 timeout 路径的边缘竞态。
    result.timed_out = false;
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
        cwd: display_cwd_opt(session.last_known_cwd.clone()),
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
        kill_session_tree(&mut session).await;
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
        kill_session_tree(&mut session).await;
        info!(conversation_scope = %key, "shell session closed during global cleanup");
    }
}
