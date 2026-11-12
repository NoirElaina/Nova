//! `unified_exec` 核心调度管理器（PTY + 管道双模式、增量轮询、进程树回收）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use nova_protocol::misc::BackgroundOutput;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::sync::{watch, Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

use crate::agent::sandbox::SandboxPolicy;

use super::process::{
    ExitStatusSnapshot, ManagedProcess, PtyControl, RollingOutputBuffer, SessionId, StdinSink,
    COMPLETED_JOB_TTL_MS, DEFAULT_MAX_OUTPUT_TOKENS, DEFAULT_YIELD_TIME_MS,
    DEFAULT_WRITE_STDIN_YIELD_TIME_MS, EXIT_DRAIN_MS, MAX_EMPTY_YIELD_TIME_MS,
    MAX_UNIFIED_EXEC_PROCESSES, MAX_YIELD_TIME_MS, MIN_EMPTY_YIELD_TIME_MS, MIN_YIELD_TIME_MS,
    WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS,
};
use super::vt_clean::{truncate_output_tokens, StreamingVtSanitizer, Utf8StreamDecoder};

static NEXT_SESSION_ID: AtomicI32 = AtomicI32::new(1);
static NEXT_CHUNK_SEQ: AtomicI32 = AtomicI32::new(1);
static PROCESS_REGISTRY: OnceLock<StdMutex<HashMap<SessionId, Arc<ManagedProcess>>>> =
    OnceLock::new();

fn registry() -> &'static StdMutex<HashMap<SessionId, Arc<ManagedProcess>>> {
    PROCESS_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

pub fn scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_string()
}

fn next_chunk_id() -> String {
    let seq = NEXT_CHUNK_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{:06x}", (seq as u32) & 0x00ff_ffff)
}

#[derive(Debug, Clone)]
pub struct ExecSandboxConfig {
    pub policy: SandboxPolicy,
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ExecCommandRequest {
    pub conversation_id: Option<String>,
    pub cmd: String,
    pub workdir: PathBuf,
    pub shell: Option<String>,
    pub login: bool,
    pub tty: bool,
    pub yield_time_ms: Option<u64>,
    pub max_output_tokens: Option<usize>,
    pub unsandboxed_override: bool,
    pub sandbox: Option<ExecSandboxConfig>,
}

fn resolve_exec_sandbox(req_sandbox: Option<&ExecSandboxConfig>, workdir: &Path) -> ExecSandboxConfig {
    if let Some(cfg) = req_sandbox {
        return cfg.clone();
    }
    ExecSandboxConfig {
        policy: crate::agent::sandbox::active_policy(),
        workspace: workdir.to_path_buf(),
        data_dir: std::env::temp_dir().join("nova-sandbox"),
    }
}

#[derive(Debug, Clone)]
pub struct WriteStdinRequest {
    pub conversation_id: Option<String>,
    pub session_id: SessionId,
    pub chars: String,
    pub yield_time_ms: Option<u64>,
    pub max_output_tokens: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ExecToolResponse {
    pub chunk_id: String,
    pub wall_time_seconds: f64,
    pub exit_code: Option<i32>,
    pub session_id: Option<SessionId>,
    pub killed: bool,
    pub output: String,
    pub original_token_count: Option<usize>,
    pub sandbox_retried: bool,
}

impl ExecToolResponse {
    pub fn format_text(&self) -> String {
        let mut lines = Vec::with_capacity(6);
        lines.push(format!("Chunk ID: {}", self.chunk_id));
        lines.push(format!("Wall time: {:.4} seconds", self.wall_time_seconds));
        if let Some(id) = self.session_id {
            lines.push(format!("Process running with session ID {}", id));
        } else if self.killed && self.exit_code.is_none() {
            lines.push("Process terminated".to_string());
        } else {
            lines.push(format!(
                "Process exited with code {}",
                self.exit_code.unwrap_or(0)
            ));
        }
        if self.sandbox_retried {
            lines.push("Sandbox: retried outside sandbox after approval".to_string());
        }
        if let Some(tokens) = self.original_token_count {
            lines.push(format!("Original token count: {}", tokens));
        }
        lines.push("Output:".to_string());
        lines.push(self.output.clone());
        lines.join("\n")
    }
}

#[derive(Debug, Clone)]
pub enum ExecError {
    InvalidInput(String),
    Cancelled,
    Execution(String),
}

impl ExecError {
    pub fn message(&self) -> &str {
        match self {
            ExecError::InvalidInput(m) | ExecError::Execution(m) => m,
            ExecError::Cancelled => "Command cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    PowerShell,
    Cmd,
    PosixShell,
}

fn classify_shell_path(path: &Path) -> ShellKind {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name == "pwsh" || name == "powershell" {
        ShellKind::PowerShell
    } else if name == "cmd" {
        ShellKind::Cmd
    } else {
        ShellKind::PosixShell
    }
}

fn find_executable_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        if !name.to_ascii_lowercase().ends_with(".exe") {
            let with_exe = dir.join(format!("{name}.exe"));
            if with_exe.is_file() {
                return Some(with_exe);
            }
        }
    }
    None
}

fn resolve_shell_program(explicit_shell: Option<&str>) -> Result<(PathBuf, ShellKind), ExecError> {
    if let Some(raw) = explicit_shell.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = raw.to_ascii_lowercase();
        let as_path = Path::new(raw);
        if as_path.is_file() {
            return Ok((as_path.to_path_buf(), classify_shell_path(as_path)));
        }

        // 先匹配已探测本机终端（支持 "pwsh", "powershell", "cmd", "git-bash", "bash", "zsh", "sh"）
        let terminals = crate::services::terminal_detector::list_available_terminals();
        if let Some(term) = terminals.into_iter().find(|t| {
            t.is_available
                && (t.id.eq_ignore_ascii_case(&lower)
                    || Path::new(&t.path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|stem| stem.eq_ignore_ascii_case(&lower))
                        .unwrap_or(false))
        }) {
            let p = PathBuf::from(&term.path);
            return Ok((p.clone(), classify_shell_path(&p)));
        }

        if let Some(found) = find_executable_in_path(raw) {
            let kind = classify_shell_path(&found);
            return Ok((found, kind));
        }

        return Err(ExecError::InvalidInput(format!(
            "Requested shell '{}' is not installed or not found on PATH",
            raw
        )));
    }

    #[cfg(target_os = "windows")]
    {
        let p = crate::services::terminal_detector::resolve_powershell_path();
        let kind = classify_shell_path(&p);
        Ok((p, kind))
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(shell_env) = std::env::var("SHELL") {
            let p = PathBuf::from(shell_env.trim());
            if p.is_file() {
                let kind = classify_shell_path(&p);
                return Ok((p, kind));
            }
        }
        for candidate in ["/bin/bash", "/usr/bin/bash", "/bin/sh"] {
            let p = PathBuf::from(candidate);
            if p.is_file() {
                return Ok((p, ShellKind::PosixShell));
            }
        }
        Ok((PathBuf::from("sh"), ShellKind::PosixShell))
    }
}

fn build_shell_args(kind: ShellKind, cmd: &str, login: bool) -> Vec<String> {
    match kind {
        ShellKind::PowerShell => {
            // 统一 UTF-8 I/O 并传播原生命令 $LASTEXITCODE 与 cmdlet $? 失败状态
            let script = format!(
                "[Console]::InputEncoding = [System.Text.Encoding]::UTF8\n\
                 [Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n\
                 $OutputEncoding = [System.Text.Encoding]::UTF8\n\
                 if ($PSStyle) {{ $PSStyle.OutputRendering = 'PlainText' }}\n\
                 $global:LASTEXITCODE = $null\n\
                 {cmd}\n\
                 if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {{ exit ([int]$LASTEXITCODE) }}\n\
                 if (-not $?) {{ exit 1 }}\n"
            );
            let mut utf16 = Vec::with_capacity(script.len() * 2);
            for unit in script.encode_utf16() {
                utf16.extend_from_slice(&unit.to_le_bytes());
            }
            let encoded = BASE64.encode(utf16);
            let mut args = vec!["-NoLogo".to_string()];
            // 只有 `login=false` 才跳过 profile；`login` 默认 true 时
            // 必须让用户 profile 生效，否则 alias / 自定义函数在会话中无法加载。
            if !login {
                args.push("-NoProfile".to_string());
            }
            args.push("-EncodedCommand".to_string());
            args.push(encoded);
            args
        }
        ShellKind::Cmd => {
            vec!["/C".to_string(), cmd.to_string()]
        }
        ShellKind::PosixShell => {
            let flag = if login { "-lc" } else { "-c" };
            vec![flag.to_string(), cmd.to_string()]
        }
    }
}

fn wrap_for_sandbox(
    program: &Path,
    args: &[String],
    sandbox: &ExecSandboxConfig,
    unsandboxed_override: bool,
) -> Result<(String, Vec<String>), ExecError> {
    let program_str = program.to_string_lossy().to_string();
    if unsandboxed_override || cfg!(windows) {
        return Ok((program_str, args.to_vec()));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let workspace_str = sandbox.workspace.to_string_lossy();
    let wrapped = crate::agent::sandbox::wrap_shell_with_policy(
        sandbox.policy,
        &program_str,
        &arg_refs,
        Some(&workspace_str),
    )
    .map_err(|e| ExecError::Execution(e.to_string()))?;
    Ok((wrapped.program, wrapped.args))
}

/// `exec_command` 的让出窗口：Windows 上抬到 10s 下限（ConPTY 冷启动成本）。
fn clamp_exec_yield_time(yield_time_ms: u64) -> u64 {
    let floor = if cfg!(windows) {
        WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS
    } else {
        MIN_YIELD_TIME_MS
    };
    yield_time_ms.clamp(floor, MAX_YIELD_TIME_MS)
}

/// `write_stdin` 的等待窗口：空输入（纯轮询）走 5s..5min，非空写入走 250ms..30s。
fn clamp_write_stdin_yield_time(yield_time_ms: u64, chars_empty: bool) -> u64 {
    if chars_empty {
        yield_time_ms.clamp(MIN_EMPTY_YIELD_TIME_MS, MAX_EMPTY_YIELD_TIME_MS)
    } else {
        yield_time_ms.clamp(MIN_YIELD_TIME_MS, MAX_YIELD_TIME_MS)
    }
}

fn ensure_capacity_and_prune() -> Result<(), ExecError> {
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    // 先清理已过 5 分钟 TTL 的已结束/已终止会话，避免陈旧句柄占位。
    map.retain(|_, p| {
        p.finished_age_ms()
            .map(|age| age < COMPLETED_JOB_TTL_MS)
            .unwrap_or(true)
    });
    if map.len() < MAX_UNIFIED_EXEC_PROCESSES {
        return Ok(());
    }

    // 优先回收已结束进程（按最近活跃时间从旧到新）
    let mut exited: Vec<(SessionId, Instant)> = map
        .iter()
        .filter(|(_, p)| p.has_exited())
        .map(|(id, p)| (*id, p.last_activity()))
        .collect();
    exited.sort_by_key(|(_, ts)| *ts);

    for (id, _) in exited {
        map.remove(&id);
        if map.len() < MAX_UNIFIED_EXEC_PROCESSES {
            return Ok(());
        }
    }

    let running_ids: Vec<String> = map
        .values()
        .filter(|p| !p.has_exited())
        .map(|p| p.session_id.to_string())
        .collect();
    Err(ExecError::Execution(format!(
        "Maximum concurrent exec sessions reached ({}/{}). Close or interrupt active session(s) [{}] using write_stdin before spawning new ones.",
        running_ids.len(),
        MAX_UNIFIED_EXEC_PROCESSES,
        running_ids.join(", ")
    )))
}

fn spawn_pty_reader_thread(
    session_id: SessionId,
    mut reader: Box<dyn std::io::Read + Send>,
    writer_arc: Arc<StdMutex<Option<Box<dyn std::io::Write + Send>>>>,
    proc_for_reader: Arc<ManagedProcess>,
    reader_done_tx: watch::Sender<bool>,
) -> Result<(), ExecError> {
    std::thread::Builder::new()
        .name(format!("nova-uexec-pty-r-{session_id}"))
        .spawn(move || {
            use std::io::{Read, Write};
            let mut decoder = Utf8StreamDecoder::default();
            let mut sanitizer = StreamingVtSanitizer::default();
            let mut raw = [0u8; 8192];

            loop {
                match reader.read(&mut raw) {
                    Ok(0) => break,
                    Ok(n) => {
                        let decoded = decoder.push_bytes(&raw[..n]);
                        if !decoded.is_empty() {
                            let cleaned = sanitizer.push_str(&decoded);
                            let replies = sanitizer.take_terminal_replies();
                            if !replies.is_empty() {
                                if let Ok(mut w_guard) = writer_arc.lock() {
                                    if let Some(ref mut w) = *w_guard {
                                        for reply in replies {
                                            let _ = w.write_all(reply);
                                        }
                                        let _ = w.flush();
                                    }
                                }
                            }
                            if !cleaned.is_empty() {
                                {
                                    let mut buf = proc_for_reader
                                        .buffer
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    buf.push_str(&cleaned);
                                }
                                proc_for_reader.touch_activity();
                                proc_for_reader.output_notify.notify_waiters();
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }

            let tail_utf8 = decoder.finish();
            let mut tail_clean = String::new();
            if !tail_utf8.is_empty() {
                tail_clean.push_str(&sanitizer.push_str(&tail_utf8));
            }
            tail_clean.push_str(&sanitizer.finish());
            if !tail_clean.is_empty() {
                let mut buf = proc_for_reader
                    .buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                buf.push_str(&tail_clean);
            }

            let _ = reader_done_tx.send(true);
            proc_for_reader.output_notify.notify_waiters();
        })
        .map(|_| ())
        .map_err(|e| ExecError::Execution(format!("Failed to start PTY reader thread: {e}")))
}

fn spawn_pty_waiter_thread<F>(
    session_id: SessionId,
    proc_for_waiter: Arc<ManagedProcess>,
    exit_tx: watch::Sender<Option<ExitStatusSnapshot>>,
    wait_fn: F,
) -> Result<(), ExecError>
where
    F: FnOnce() -> Option<i32> + Send + 'static,
{
    std::thread::Builder::new()
        .name(format!("nova-uexec-pty-w-{session_id}"))
        .spawn(move || {
            let exit_code = wait_fn();
            let killed = proc_for_waiter.killed_flag.load(Ordering::SeqCst);
            let _ = exit_tx.send(Some(ExitStatusSnapshot {
                exit_code,
                killed,
                timed_out: false,
                finished_at: Instant::now(),
            }));
            proc_for_waiter.touch_activity();
            proc_for_waiter.output_notify.notify_waiters();

            // 最多等待 EXIT_DRAIN_MS 让读取线程排干末尾输出；超时后释放 master / ConPTY 避免读取线程泄漏。
            let drain_start = Instant::now();
            while !*proc_for_waiter.reader_done_rx.borrow()
                && drain_start.elapsed() < Duration::from_millis(EXIT_DRAIN_MS)
            {
                std::thread::sleep(Duration::from_millis(8));
            }
            if let Ok(mut ctrl) = proc_for_waiter.pty_control.lock() {
                if let Some(mut c) = ctrl.take() {
                    c.killer = None;
                    c.master = None;
                    #[cfg(windows)]
                    {
                        c.win_conpty = None;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|e| ExecError::Execution(format!("Failed to start PTY waiter thread: {e}")))
}

enum PtyBackend {
    Portable {
        master: Box<dyn portable_pty::MasterPty + Send>,
        child: Box<dyn portable_pty::Child + Send + Sync>,
        killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    },
    #[cfg(windows)]
    WindowsSandbox {
        conpty: crate::agent::sandbox::windows::WinConPty,
        job: crate::agent::sandbox::windows::WinJobObject,
        child_process: std::os::windows::io::OwnedHandle,
    },
}

const PTY_BASE_ENV: [(&str, &str); 5] = [
    ("TERM", "xterm-256color"),
    ("NO_COLOR", "1"),
    ("CLICOLOR", "0"),
    ("PYTHONIOENCODING", "utf-8"),
    ("PYTHONUNBUFFERED", "1"),
];

struct PtySpawnOutput {
    backend: PtyBackend,
    pid: u32,
    reader: Box<dyn std::io::Read + Send>,
    writer: Box<dyn std::io::Write + Send>,
}

struct SpawnProcessSpec {
    session_id: SessionId,
    scope: String,
    cmd_text: String,
    final_program: String,
    final_args: Vec<String>,
    workdir: PathBuf,
    sandbox: ExecSandboxConfig,
    unsandboxed_override: bool,
}

async fn spawn_pty_process(spec: SpawnProcessSpec) -> Result<Arc<ManagedProcess>, ExecError> {
    let SpawnProcessSpec {
        session_id,
        scope,
        cmd_text,
        final_program,
        final_args,
        workdir,
        sandbox,
        unsandboxed_override,
    } = spec;
    let workdir_display = workdir.display().to_string();
    let shell_display = final_program.clone();
    let use_win_sandbox = cfg!(windows) && sandbox.policy.is_enforcing() && !unsandboxed_override;

    let spawn_result = tokio::task::spawn_blocking(
        move || -> Result<PtySpawnOutput, String> {
            #[cfg(windows)]
            if use_win_sandbox {
                let arg_refs: Vec<&str> = final_args.iter().map(String::as_str).collect();
                let spawned = crate::agent::sandbox::windows::spawn_sandboxed_conpty(
                    sandbox.policy,
                    &sandbox.data_dir,
                    &sandbox.workspace,
                    &workdir,
                    &final_program,
                    &arg_refs,
                    &PTY_BASE_ENV,
                    120,
                    24,
                )?;
                let pid = spawned.pid;
                let reader: Box<dyn std::io::Read + Send> = Box::new(spawned.reader);
                let writer: Box<dyn std::io::Write + Send> = Box::new(spawned.writer);
                let backend = PtyBackend::WindowsSandbox {
                    conpty: spawned.conpty,
                    job: spawned.job,
                    child_process: spawned.child_process,
                };
                return Ok(PtySpawnOutput {
                    backend,
                    pid,
                    reader,
                    writer,
                });
            }

            let _ = (&sandbox, use_win_sandbox);
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows: 24,
                    cols: 120,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("Failed to allocate PTY: {e}"))?;

            let mut builder = CommandBuilder::new(&final_program);
            for arg in &final_args {
                builder.arg(arg);
            }
            builder.cwd(&workdir);
            for (k, v) in PTY_BASE_ENV {
                builder.env(k, v);
            }

            let child = pair
                .slave
                .spawn_command(builder)
                .map_err(|e| format!("Failed to spawn command inside PTY: {e}"))?;
            // 关键：立刻释放 slave 句柄，否则子进程退出后 reader 无法收到 EOF！
            drop(pair.slave);

            let pid = child.process_id().unwrap_or(0);
            let killer = child.clone_killer();
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| format!("Failed to clone PTY reader: {e}"))?;
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| format!("Failed to acquire PTY writer: {e}"))?;

            Ok(PtySpawnOutput {
                backend: PtyBackend::Portable {
                    master: pair.master,
                    child,
                    killer,
                },
                pid,
                reader,
                writer,
            })
        },
    )
    .await
    .map_err(|e| ExecError::Execution(format!("PTY spawn task panicked: {e}")))?
    .map_err(ExecError::Execution)?;

    let PtySpawnOutput {
        backend,
        pid,
        reader,
        writer,
    } = spawn_result;

    let buffer = Arc::new(StdMutex::new(RollingOutputBuffer::default()));
    let output_notify = Arc::new(Notify::new());
    let (exit_tx, exit_rx) = watch::channel(None);
    let (reader_done_tx, reader_done_rx) = watch::channel(false);

    enum PtyWaiter {
        Portable(Box<dyn portable_pty::Child + Send + Sync>),
        #[cfg(windows)]
        Windows(std::os::windows::io::OwnedHandle),
    }

    let (pty_control, win_job, waiter_kind) = match backend {
        PtyBackend::Portable {
            master,
            child,
            killer,
        } => {
            #[cfg(windows)]
            let no_job: Option<crate::agent::sandbox::windows::WinJobObject> = None;
            #[cfg(not(windows))]
            let no_job = ();
            (
                PtyControl {
                    killer: Some(killer),
                    master: Some(master),
                    #[cfg(windows)]
                    win_conpty: None,
                },
                no_job,
                PtyWaiter::Portable(child),
            )
        }
        #[cfg(windows)]
        PtyBackend::WindowsSandbox {
            conpty,
            job,
            child_process,
        } => (
            PtyControl {
                killer: None,
                master: None,
                win_conpty: Some(conpty),
            },
            Some(job),
            PtyWaiter::Windows(child_process),
        ),
    };
    #[cfg(not(windows))]
    let _ = win_job;

    let writer_arc = Arc::new(StdMutex::new(Some(writer)));
    let process = Arc::new(ManagedProcess {
        session_id,
        scope,
        pid,
        command: cmd_text,
        workdir: workdir_display,
        shell: shell_display,
        tty: true,
        started_at: Instant::now(),
        last_activity_at: StdMutex::new(Instant::now()),
        yielded_to_background: AtomicBool::new(false),
        buffer: Arc::clone(&buffer),
        model_cursor: StdMutex::new(0),
        stdin: Arc::new(StdinSink::Pty(Arc::clone(&writer_arc))),
        interaction_lock: Arc::new(AsyncMutex::new(())),
        output_notify: Arc::clone(&output_notify),
        exit_tx: exit_tx.clone(),
        exit_rx,
        reader_done_rx,
        pty_control: StdMutex::new(Some(pty_control)),
        #[cfg(windows)]
        win_job: StdMutex::new(win_job),
        killed_flag: AtomicBool::new(false),
    });

    spawn_pty_reader_thread(
        session_id,
        reader,
        writer_arc,
        Arc::clone(&process),
        reader_done_tx,
    )?;

    spawn_pty_waiter_thread(session_id, Arc::clone(&process), exit_tx, move || match waiter_kind {
        PtyWaiter::Portable(mut child) => child.wait().ok().map(|s| s.exit_code() as i32),
        #[cfg(windows)]
        PtyWaiter::Windows(child_process) => {
            Some(crate::agent::sandbox::windows::wait_process_exit(&child_process))
        }
    })?;

    Ok(process)
}

async fn pump_pipe_stream<R>(
    mut reader: R,
    proc: Arc<ManagedProcess>,
    pending_streams: Arc<AtomicU8>,
    reader_done_tx: watch::Sender<bool>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut decoder = Utf8StreamDecoder::default();
    let mut sanitizer = StreamingVtSanitizer::default();
    let mut raw = [0u8; 8192];

    loop {
        match reader.read(&mut raw).await {
            Ok(0) => break,
            Ok(n) => {
                let decoded = decoder.push_bytes(&raw[..n]);
                if !decoded.is_empty() {
                    let cleaned = sanitizer.push_str(&decoded);
                    if !cleaned.is_empty() {
                        {
                            let mut buf = proc.buffer.lock().unwrap_or_else(|e| e.into_inner());
                            buf.push_str(&cleaned);
                        }
                        proc.touch_activity();
                        proc.output_notify.notify_waiters();
                    }
                }
            }
            Err(_) => break,
        }
    }

    let tail_utf8 = decoder.finish();
    let mut tail_clean = String::new();
    if !tail_utf8.is_empty() {
        tail_clean.push_str(&sanitizer.push_str(&tail_utf8));
    }
    tail_clean.push_str(&sanitizer.finish());
    if !tail_clean.is_empty() {
        let mut buf = proc.buffer.lock().unwrap_or_else(|e| e.into_inner());
        buf.push_str(&tail_clean);
    }

    if pending_streams.fetch_sub(1, Ordering::AcqRel) == 1 {
        let _ = reader_done_tx.send(true);
        proc.output_notify.notify_waiters();
    }
}

const PIPE_BASE_ENV: [(&str, &str); 4] = [
    ("NO_COLOR", "1"),
    ("CLICOLOR", "0"),
    ("PYTHONIOENCODING", "utf-8"),
    ("PYTHONUNBUFFERED", "1"),
];

enum PipeWaiter {
    Child(Box<tokio::process::Child>),
    #[cfg(windows)]
    WindowsHandle(std::os::windows::io::OwnedHandle),
}

async fn spawn_pipe_process(spec: SpawnProcessSpec) -> Result<Arc<ManagedProcess>, ExecError> {
    let SpawnProcessSpec {
        session_id,
        scope,
        cmd_text,
        final_program,
        final_args,
        workdir,
        sandbox,
        unsandboxed_override,
    } = spec;
    let use_win_sandbox = cfg!(windows) && sandbox.policy.is_enforcing() && !unsandboxed_override;

    #[cfg(windows)]
    type SpawnPipeTuple = (
        u32,
        Option<Box<dyn AsyncWrite + Send + Unpin>>,
        Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
        Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
        Option<crate::agent::sandbox::windows::WinJobObject>,
        PipeWaiter,
    );
    #[cfg(not(windows))]
    type SpawnPipeTuple = (
        u32,
        Option<Box<dyn AsyncWrite + Send + Unpin>>,
        Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
        Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
        (),
        PipeWaiter,
    );

    let (pid, stdin, stdout, stderr, win_job, waiter): SpawnPipeTuple = {
        #[cfg(windows)]
        if use_win_sandbox {
            let workdir_clone = workdir.clone();
            let program_clone = final_program.clone();
            let args_clone = final_args.clone();
            let spawned = tokio::task::spawn_blocking(move || {
                let arg_refs: Vec<&str> = args_clone.iter().map(String::as_str).collect();
                crate::agent::sandbox::windows::spawn_sandboxed_pipe(
                    sandbox.policy,
                    &sandbox.data_dir,
                    &sandbox.workspace,
                    &workdir_clone,
                    &program_clone,
                    &arg_refs,
                    &PIPE_BASE_ENV,
                )
            })
            .await
            .map_err(|e| ExecError::Execution(format!("Pipe sandbox task panicked: {e}")))?
            .map_err(ExecError::Execution)?;

            let stdin_box: Box<dyn AsyncWrite + Send + Unpin> =
                Box::new(tokio::fs::File::from_std(spawned.stdin));
            let stdout_box: Box<dyn tokio::io::AsyncRead + Unpin + Send> =
                Box::new(tokio::fs::File::from_std(spawned.stdout));
            let stderr_box: Box<dyn tokio::io::AsyncRead + Unpin + Send> =
                Box::new(tokio::fs::File::from_std(spawned.stderr));
            (
                spawned.pid,
                Some(stdin_box),
                Some(stdout_box),
                Some(stderr_box),
                Some(spawned.job),
                PipeWaiter::WindowsHandle(spawned.child_process),
            )
        } else {
            let mut cmd = tokio::process::Command::new(&final_program);
            cmd.args(&final_args)
                .current_dir(&workdir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for (k, v) in PIPE_BASE_ENV {
                cmd.env(k, v);
            }
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);

            let mut child = cmd
                .spawn()
                .map_err(|e| ExecError::Execution(format!("Failed to spawn process: {e}")))?;
            let pid = child.id().unwrap_or(0);
            let stdin = child
                .stdin
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncWrite + Send + Unpin>);
            let stdout = child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
            let stderr = child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
            (pid, stdin, stdout, stderr, None, PipeWaiter::Child(Box::new(child)))
        }

        #[cfg(not(windows))]
        {
            let _ = (&sandbox, use_win_sandbox);
            let mut cmd = tokio::process::Command::new(&final_program);
            cmd.args(&final_args)
                .current_dir(&workdir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for (k, v) in PIPE_BASE_ENV {
                cmd.env(k, v);
            }
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }

            let mut child = cmd
                .spawn()
                .map_err(|e| ExecError::Execution(format!("Failed to spawn process: {e}")))?;
            let pid = child.id().unwrap_or(0);
            let stdin = child
                .stdin
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncWrite + Send + Unpin>);
            let stdout = child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
            let stderr = child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
            (pid, stdin, stdout, stderr, (), PipeWaiter::Child(Box::new(child)))
        }
    };
    #[cfg(not(windows))]
    let _ = win_job;

    let buffer = Arc::new(StdMutex::new(RollingOutputBuffer::default()));
    let output_notify = Arc::new(Notify::new());
    let (exit_tx, exit_rx) = watch::channel(None);
    let (reader_done_tx, reader_done_rx) = watch::channel(stdout.is_none() && stderr.is_none());

    let process = Arc::new(ManagedProcess {
        session_id,
        scope,
        pid,
        command: cmd_text,
        workdir: workdir.display().to_string(),
        shell: final_program,
        tty: false,
        started_at: Instant::now(),
        last_activity_at: StdMutex::new(Instant::now()),
        yielded_to_background: AtomicBool::new(false),
        buffer,
        model_cursor: StdMutex::new(0),
        stdin: Arc::new(StdinSink::Pipe(Arc::new(AsyncMutex::new(stdin)))),
        interaction_lock: Arc::new(AsyncMutex::new(())),
        output_notify,
        exit_tx: exit_tx.clone(),
        exit_rx,
        reader_done_rx,
        pty_control: StdMutex::new(None),
        #[cfg(windows)]
        win_job: StdMutex::new(win_job),
        killed_flag: AtomicBool::new(false),
    });

    let stream_count = (stdout.is_some() as u8) + (stderr.is_some() as u8);
    let pending_streams = Arc::new(AtomicU8::new(stream_count));
    if let Some(out) = stdout {
        tokio::spawn(pump_pipe_stream(
            out,
            Arc::clone(&process),
            Arc::clone(&pending_streams),
            reader_done_tx.clone(),
        ));
    }
    if let Some(err) = stderr {
        tokio::spawn(pump_pipe_stream(
            err,
            Arc::clone(&process),
            pending_streams,
            reader_done_tx,
        ));
    }

    let proc_for_waiter = Arc::clone(&process);
    tokio::spawn(async move {
        let exit_code = match waiter {
            PipeWaiter::Child(mut child) => child.wait().await.ok().and_then(|s| s.code()),
            #[cfg(windows)]
            PipeWaiter::WindowsHandle(child_process) => tokio::task::spawn_blocking(move || {
                crate::agent::sandbox::windows::wait_process_exit(&child_process)
            })
            .await
            .ok(),
        };
        let killed = proc_for_waiter.killed_flag.load(Ordering::SeqCst);
        let _ = exit_tx.send(Some(ExitStatusSnapshot {
            exit_code,
            killed,
            timed_out: false,
            finished_at: Instant::now(),
        }));
        proc_for_waiter.touch_activity();
        proc_for_waiter.output_notify.notify_waiters();
    });

    Ok(process)
}

async fn collect_until_yield_or_exit(
    proc: &Arc<ManagedProcess>,
    yield_time: Duration,
    max_output_tokens: usize,
    cancel_token: Option<&CancellationToken>,
    wait_start: Instant,
) -> Result<ExecToolResponse, ExecError> {
    let deadline = tokio::time::Instant::now() + yield_time;
    let mut exit_rx = proc.exit_rx.clone();

    if exit_rx.borrow().is_none() {
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                _ = async {
                    if let Some(token) = cancel_token {
                        token.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    return Err(ExecError::Cancelled);
                }
                changed = exit_rx.changed() => {
                    if changed.is_err() || exit_rx.borrow().is_some() {
                        break;
                    }
                }
                _ = tokio::time::sleep(remaining) => {
                    break;
                }
            }
        }
    }

    // 进程已退出时等待读取流把尾部字节刷干净（最多 EXIT_DRAIN_MS，拿到 EOF 立即退出）
    if exit_rx.borrow().is_some() {
        let mut rdone = proc.reader_done_rx.clone();
        if !*rdone.borrow() {
            let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(EXIT_DRAIN_MS);
            loop {
                if *rdone.borrow() {
                    break;
                }
                let rem = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
                if rem.is_zero() {
                    break;
                }
                tokio::select! {
                    _ = rdone.changed() => {}
                    _ = tokio::time::sleep(rem) => break,
                }
            }
        }
    }

    let delta = proc.drain_model_delta();
    // 响应头（Chunk ID / Wall time / exit / token 计数）本身要占几十个 token，
    // 先给它留出预算，否则单次返回会稳定超出 max_output_tokens。
    const RESPONSE_HEADER_TOKEN_RESERVE: usize = 48;
    let truncated = truncate_output_tokens(
        &delta,
        max_output_tokens.saturating_sub(RESPONSE_HEADER_TOKEN_RESERVE).max(64),
    );
    let wall_time_seconds = wait_start.elapsed().as_secs_f64();

    if let Some(exit) = proc.exit_snapshot() {
        Ok(ExecToolResponse {
            chunk_id: next_chunk_id(),
            wall_time_seconds,
            exit_code: exit.exit_code,
            session_id: None,
            killed: exit.killed,
            output: truncated.text,
            original_token_count: truncated.truncated.then_some(truncated.approx_original_tokens),
            sandbox_retried: false,
        })
    } else {
        proc.yielded_to_background.store(true, Ordering::SeqCst);
        Ok(ExecToolResponse {
            chunk_id: next_chunk_id(),
            wall_time_seconds,
            exit_code: None,
            session_id: Some(proc.session_id),
            killed: false,
            output: truncated.text,
            original_token_count: truncated.truncated.then_some(truncated.approx_original_tokens),
            sandbox_retried: false,
        })
    }
}

/// 启动命令：默认在 TTY（`portable-pty`）中运行，并在 `yield_time_ms` 内等待完成或转入交互会话。
pub async fn exec_command(
    req: ExecCommandRequest,
    cancel_token: Option<&CancellationToken>,
) -> Result<ExecToolResponse, ExecError> {
    if req.cmd.trim().is_empty() {
        return Err(ExecError::InvalidInput("cmd must not be empty".to_string()));
    }
    if !req.workdir.is_dir() {
        return Err(ExecError::InvalidInput(format!(
            "Working directory does not exist: {}",
            req.workdir.display()
        )));
    }

    ensure_capacity_and_prune()?;

    let sandbox = resolve_exec_sandbox(req.sandbox.as_ref(), &req.workdir);
    let (shell_path, shell_kind) = resolve_shell_program(req.shell.as_deref())?;
    let args = build_shell_args(shell_kind, &req.cmd, req.login);
    let (final_program, final_args) = wrap_for_sandbox(
        &shell_path,
        &args,
        &sandbox,
        req.unsandboxed_override,
    )?;

    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let scope = scope_key(req.conversation_id.as_deref());
    let wait_start = Instant::now();
    let spec = SpawnProcessSpec {
        session_id,
        scope,
        cmd_text: req.cmd,
        final_program,
        final_args,
        workdir: req.workdir,
        sandbox,
        unsandboxed_override: req.unsandboxed_override,
    };

    let process = if req.tty {
        spawn_pty_process(spec).await?
    } else {
        spawn_pipe_process(spec).await?
    };

    {
        let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
        map.insert(session_id, Arc::clone(&process));
    }

    let yield_ms = clamp_exec_yield_time(req.yield_time_ms.unwrap_or(DEFAULT_YIELD_TIME_MS));
    let max_output_tokens = req.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

    let outcome = collect_until_yield_or_exit(
        &process,
        Duration::from_millis(yield_ms),
        max_output_tokens,
        cancel_token,
        wait_start,
    )
    .await;

    match outcome {
        Ok(resp) => {
            if resp.session_id.is_none() {
                // 在首次让出窗口内即执行结束的短命令，不污染长期交互式/后台任务表。
                {
                    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
                    map.remove(&session_id);
                }
                // 清理残留 Job / 孙进程与 ConPTY 句柄，防止非 PTY 孙进程继承句柄长期滞留。
                process.kill_tree().await;
            }
            Ok(resp)
        }
        Err(ExecError::Cancelled) => {
            process.kill_tree().await;
            let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
            map.remove(&session_id);
            Err(ExecError::Cancelled)
        }
        Err(other) => {
            let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
            map.remove(&session_id);
            Err(other)
        }
    }
}

/// 向活跃会话写入 stdin 并收集接下来的输出增量；传空 `chars` 则仅等待并轮询增量输出。
pub async fn write_stdin(
    req: WriteStdinRequest,
    cancel_token: Option<&CancellationToken>,
) -> Result<ExecToolResponse, ExecError> {
    let caller_scope = scope_key(req.conversation_id.as_deref());
    let proc = {
        let map = registry().lock().unwrap_or_else(|e| e.into_inner());
        map.get(&req.session_id).cloned()
    }
    .ok_or_else(|| {
        ExecError::InvalidInput(format!(
            "Unknown or already closed session_id: {}",
            req.session_id
        ))
    })?;

    if req.conversation_id.is_some() && proc.scope != caller_scope {
        return Err(ExecError::InvalidInput(format!(
            "Session {} belongs to another conversation scope",
            req.session_id
        )));
    }

    let _interaction_guard = proc.interaction_lock.lock().await;

    let wait_start = Instant::now();
    let chars_empty = req.chars.is_empty();
    let yield_ms = clamp_write_stdin_yield_time(
        req.yield_time_ms
            .unwrap_or(DEFAULT_WRITE_STDIN_YIELD_TIME_MS),
        chars_empty,
    );
    let max_output_tokens = req.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

    if !req.chars.is_empty() {
        if let Some(exit) = proc.exit_snapshot() {
            // 进程已在本次 write_stdin 前退出：返回残余输出与清晰退出信息，并提示重启会话。
            let remaining = proc.drain_model_delta();
            return Err(ExecError::InvalidInput(format!(
                "Session {} has already exited (exit_code={:?}). Cannot write stdin.{}{}",
                req.session_id,
                exit.exit_code,
                if remaining.is_empty() { "" } else { "\nRemaining output:\n" },
                remaining
            )));
        }
        proc.touch_activity();

        if !proc.tty {
            // 非 TTY（纯管道）无终端行规程：仅 "\u{3}" 触发中断并结束进程；
            // 其它任意非空写入一律以 InvalidInput 拒绝。
            if req.chars == "\u{3}" {
                proc.kill_tree().await;
            } else {
                return Err(ExecError::InvalidInput(format!(
                    "stdin is closed for session {}; rerun exec_command with tty=true to keep stdin open",
                    req.session_id
                )));
            }
        } else {
            // TTY 会话：包括 "\u{3}" (0x03) 在内按字节写入 PTY，
            // 让交互式 shell 只中断前台任务而不销毁外层会话。
            let payload_bytes: Vec<u8> = if cfg!(target_os = "windows") {
                // Windows ConPTY 仅把 CR(\r) 识别为 Enter 键提交输入；0x03 原样透传。
                req.chars.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
            } else {
                req.chars.into_bytes()
            };

            if let Err(err) = proc.stdin.write_bytes(&payload_bytes).await {
                if !proc.has_exited() {
                    return Err(ExecError::Execution(err));
                }
            }
        }
    }

    let resp = collect_until_yield_or_exit(
        &proc,
        Duration::from_millis(yield_ms),
        max_output_tokens,
        cancel_token,
        wait_start,
    )
    .await?;

    Ok(resp)
}

pub fn list_processes_for_conversation(conversation_id: Option<&str>) -> Vec<BackgroundOutput> {
    let key = scope_key(conversation_id);
    let map = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut items: Vec<(SessionId, BackgroundOutput)> = map
        .values()
        .filter(|p| p.scope == key && p.yielded_to_background.load(Ordering::Relaxed))
        // 已结束的会话只在 TTL 内保留展示，避免"0/5 全部已完成"的陈旧列表长期堆积。
        .filter(|p| match p.finished_age_ms() {
            Some(age) => age < COMPLETED_JOB_TTL_MS,
            None => true,
        })
        .map(|p| (p.session_id, p.to_background_output(false, 0)))
        .collect();
    items.sort_by_key(|(id, _)| *id);
    items.into_iter().map(|(_, item)| item).collect()
}

pub fn read_process_snapshot(
    conversation_id: Option<&str>,
    id: &str,
    tail_lines: Option<usize>,
) -> Result<BackgroundOutput, String> {
    let session_id = id
        .trim()
        .parse::<SessionId>()
        .map_err(|_| format!("Invalid exec session id: '{}'", id))?;
    let proc = {
        let map = registry().lock().unwrap_or_else(|e| e.into_inner());
        map.get(&session_id).cloned()
    }
    .ok_or_else(|| format!("Exec session '{}' not found", id))?;

    if let Some(conv) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
        if proc.scope != conv {
            return Err(format!("Exec session '{}' does not belong to this conversation", id));
        }
    }

    Ok(proc.to_background_output(true, tail_lines.unwrap_or(400)))
}

pub async fn kill_process(
    conversation_id: Option<&str>,
    id: &str,
) -> Result<BackgroundOutput, String> {
    let session_id = id
        .trim()
        .parse::<SessionId>()
        .map_err(|_| format!("Invalid exec session id: '{}'", id))?;
    let proc = {
        let map = registry().lock().unwrap_or_else(|e| e.into_inner());
        map.get(&session_id).cloned()
    }
    .ok_or_else(|| format!("Exec session '{}' not found", id))?;

    if let Some(conv) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
        if proc.scope != conv {
            return Err(format!("Exec session '{}' does not belong to this conversation", id));
        }
    }

    proc.kill_tree().await;
    let snapshot = proc.to_background_output(true, 200);
    // 与自然退出保持一致：保留在注册表内直到 COMPLETED_JOB_TTL_MS 过期（或在超上限时由 ensure_capacity_and_prune 先淘汰），
    // 使 UI 仍能展示刚手动停止任务的终态与最后输出。
    Ok(snapshot)
}

pub async fn close_conversation(conversation_id: Option<&str>) {
    let key = scope_key(conversation_id);
    let to_kill: Vec<Arc<ManagedProcess>> = {
        let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<SessionId> = map
            .iter()
            .filter(|(_, p)| p.scope == key)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    };

    for proc in to_kill {
        proc.kill_tree().await;
    }
}

pub async fn close_all() {
    let all: Vec<Arc<ManagedProcess>> = {
        let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
        map.drain().map(|(_, p)| p).collect()
    };
    for proc in all {
        proc.kill_tree().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_workdir() -> PathBuf {
        std::env::temp_dir()
    }

    fn unique_scope(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{prefix}-{nanos}-{}", NEXT_CHUNK_SEQ.fetch_add(1, Ordering::Relaxed))
    }

    #[tokio::test]
    async fn exec_command_fast_completion_pty_and_pipe() {
        let scope_pty = unique_scope("fast-pty");
        let resp_pty = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope_pty.clone()),
                cmd: if cfg!(target_os = "windows") {
                    "Write-Output 'nova-pty-ok:中文输出'".to_string()
                } else {
                    "echo 'nova-pty-ok:中文输出'".to_string()
                },
                workdir: test_workdir(),
                shell: None,
                login: true,
                tty: true,
                yield_time_ms: Some(10_000),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("pty exec should succeed");

        assert_eq!(resp_pty.exit_code, Some(0));
        assert!(resp_pty.session_id.is_none());
        assert!(
            resp_pty.output.contains("nova-pty-ok:"),
            "unexpected pty output: {:?}",
            resp_pty.output
        );
        // 不应包含 ConPTY 原始设置控制符
        assert!(!resp_pty.output.contains("\x1b["), "raw ANSI leaked: {:?}", resp_pty.output);
        // 短命命令不应留存于 UI 后台芯片列表中
        assert!(list_processes_for_conversation(Some(&scope_pty)).is_empty());

        let scope_pipe = unique_scope("fast-pipe");
        let resp_pipe = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope_pipe),
                cmd: if cfg!(target_os = "windows") {
                    "Write-Output 'nova-pipe-ok'".to_string()
                } else {
                    "echo 'nova-pipe-ok'".to_string()
                },
                workdir: test_workdir(),
                shell: None,
                login: true,
                tty: false,
                yield_time_ms: Some(10_000),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("pipe exec should succeed");

        assert_eq!(resp_pipe.exit_code, Some(0));
        assert!(resp_pipe.session_id.is_none());
        assert!(resp_pipe.output.contains("nova-pipe-ok"));
    }

    #[tokio::test]
    async fn exec_command_propagates_nonzero_exit_code() {
        let scope = unique_scope("exit-code");
        let resp = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope),
                cmd: if cfg!(target_os = "windows") {
                    "cmd /c exit 7".to_string()
                } else {
                    "exit 7".to_string()
                },
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: false,
                yield_time_ms: Some(5000),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("nonzero command should return ExecToolResponse");

        assert_eq!(resp.exit_code, Some(7));
        assert!(resp.session_id.is_none());
    }

    #[tokio::test]
    async fn interactive_pty_yield_write_stdin_incremental_and_exit() {
        let scope = unique_scope("interactive-pty");
        // 启动一个持久交互脚本：回显每行输入直到收到 quit
        let cmd = if cfg!(target_os = "windows") {
            r#"Write-Output "READY"; while ($true) { $line = [Console]::In.ReadLine(); if ($null -eq $line -or $line -eq "quit") { break }; Write-Output ("ECHO:" + $line) }"#.to_string()
        } else {
            r#"echo "READY"; while IFS= read -r line; do [ "$line" = "quit" ] && break; echo "ECHO:$line"; done"#.to_string()
        };

        let first = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope.clone()),
                cmd,
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: true,
                yield_time_ms: Some(1200),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("interactive spawn failed");

        let sid = first
            .session_id
            .expect("interactive process should yield active session_id");
        assert!(
            first.output.contains("READY"),
            "expected READY banner, got {:?}",
            first.output
        );

        // UI 可见运行中的作业，且 UI 读取绝不会消耗模型游标
        let listed = list_processes_for_conversation(Some(&scope));
        assert_eq!(listed.len(), 1);
        assert!(listed[0].running);
        let ui_snap = read_process_snapshot(Some(&scope), &sid.to_string(), Some(50))
            .expect("snapshot should succeed");
        assert!(ui_snap.stdout.contains("READY"));

        // 发送交互输入 ping，读取增量输出（不应再重复包含 READY）
        let turn2 = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: "ping_one\n".to_string(),
                yield_time_ms: Some(800),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect("write_stdin failed");

        assert_eq!(turn2.session_id, Some(sid));
        assert!(
            turn2.output.contains("ECHO:ping_one"),
            "expected ECHO:ping_one, got {:?}",
            turn2.output
        );
        assert!(
            !turn2.output.contains("READY"),
            "delta must not repeat already consumed READY: {:?}",
            turn2.output
        );

        // 发送 quit 让交互会话自然退出
        let turn3 = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: "quit\n".to_string(),
                yield_time_ms: Some(1500),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect("write_stdin quit failed");

        assert!(
            turn3.session_id.is_none(),
            "expected session to exit, got session_id={:?}, output={:?}",
            turn3.session_id,
            turn3.output
        );
        assert_eq!(turn3.exit_code, Some(0));

        close_conversation(Some(&scope)).await;
        assert!(list_processes_for_conversation(Some(&scope)).is_empty());
    }

    #[tokio::test]
    async fn ctrl_c_interrupts_foreground_command_in_tty_shell_without_killing_session() {
        let scope = unique_scope("ctrl-c-tty-fg");
        // 启动交互式 shell（而非一次性 loop 脚本），模拟用户/模型在 REPL 中用 \u{0003} 中断卡住的前台命令。
        let shell_cmd = if cfg!(target_os = "windows") {
            "pwsh -NoLogo -NoProfile".to_string()
        } else {
            "sh".to_string()
        };

        let spawned = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope.clone()),
                cmd: shell_cmd,
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: true,
                yield_time_ms: Some(if cfg!(windows) { 10_000 } else { 1000 }),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("spawn interactive shell failed");

        let sid = spawned.session_id.expect("interactive shell should yield session_id");

        // Windows 在 cargo test 下会继承忽略 Ctrl+C 属性：只在最内层前台子进程内复位 SetConsoleCtrlHandler(0, false)，
        // 并用 -EncodedCommand 避免引号嵌套问题且杜绝 PTY 回显伪命中 FG_42。
        let fg_cmd = if cfg!(target_os = "windows") {
            let child_script = concat!(
                "Add-Type 'using System; using System.Runtime.InteropServices; ",
                "public static class CtrlCFix { [DllImport(\"kernel32.dll\")] public static extern bool SetConsoleCtrlHandler(IntPtr h, bool add); }'; ",
                "[void][CtrlCFix]::SetConsoleCtrlHandler([IntPtr]::Zero, $false); ",
                "Write-Output ('FG_' + (20 + 22)); ",
                "while ($true) { Start-Sleep -Milliseconds 100 }"
            );
            let mut utf16 = Vec::with_capacity(child_script.len() * 2);
            for u in child_script.encode_utf16() {
                utf16.extend_from_slice(&u.to_le_bytes());
            }
            format!("& pwsh -NoLogo -NoProfile -EncodedCommand {}\n", BASE64.encode(utf16))
        } else {
            "sh -c 'echo \"FG_$((20 + 22))\"; while true; do sleep 0.1; done'\n".to_string()
        };

        let mut running = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: fg_cmd,
                yield_time_ms: Some(2500),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect("start foreground loop failed");
        let mut combined_fg_output = running.output.clone();
        for _ in 0..4 {
            if combined_fg_output.contains("FG_42") {
                break;
            }
            running = write_stdin(
                WriteStdinRequest {
                    conversation_id: Some(scope.clone()),
                    session_id: sid,
                    chars: String::new(),
                    yield_time_ms: Some(1500),
                    max_output_tokens: None,
                },
                None,
            )
            .await
            .expect("poll foreground output failed");
            combined_fg_output.push_str(&running.output);
        }
        assert_eq!(running.session_id, Some(sid));
        assert!(
            combined_fg_output.contains("FG_42"),
            "expected FG_42 from running foreground child, got {:?}",
            combined_fg_output
        );

        // 发送 0x03（Ctrl+C）：仅中断当前前台命令，外层交互式 shell 仍保持存活。
        let after_ctrl_c = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: "\u{0003}".to_string(),
                yield_time_ms: Some(1500),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect("write_stdin Ctrl+C failed");
        assert_eq!(
            after_ctrl_c.session_id,
            Some(sid),
            "Ctrl+C in tty=true interactive shell must keep session alive, output={:?}",
            after_ctrl_c.output
        );

        // 继续在同一会话内执行后续命令并正常退出，证明外层 shell 确实仍可用。
        // 用计算表达式 ALIVE_$(40+2) -> ALIVE_42，避免 PTY 本地回显误命中字面量。
        let next_turn = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: if cfg!(windows) {
                    "Write-Output \"ALIVE_$(40 + 2)\"\nexit 0\n".to_string()
                } else {
                    "echo \"ALIVE_$((40 + 2))\"\nexit 0\n".to_string()
                },
                yield_time_ms: Some(3000),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect("subsequent command after Ctrl+C failed");
        assert!(
            next_turn.output.contains("ALIVE_42"),
            "expected ALIVE_42, got {:?}",
            next_turn.output
        );
        assert!(next_turn.session_id.is_none());
        if cfg!(target_os = "windows") {
            // Windows ConPTY 会将 CTRL_C_EVENT 广播至同控制台组的外层 `pwsh -EncodedCommand` 宿主，
            // 内层交互 shell 在运行完 ALIVE_42 并执行 `exit 0` 后，外层宿主因早前管线曾收到中断标记而常返回 1。
            assert!(
                matches!(next_turn.exit_code, Some(0 | 1)),
                "unexpected exit_code after Ctrl+C recovery: {:?}",
                next_turn.exit_code
            );
        } else {
            assert_eq!(next_turn.exit_code, Some(0));
        }

        close_conversation(Some(&scope)).await;
    }

    #[tokio::test]
    async fn ctrl_c_terminates_pipe_process_and_rejects_arbitrary_stdin() {
        let scope = unique_scope("ctrl-c-pipe");
        let cmd = if cfg!(target_os = "windows") {
            "Write-Output 'PIPE_STARTED'; while ($true) { Start-Sleep -Milliseconds 100 }".to_string()
        } else {
            "echo 'PIPE_STARTED'; while true; do sleep 0.1; done".to_string()
        };

        let spawned = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope.clone()),
                cmd,
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: false,
                yield_time_ms: Some(1200),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("spawn pipe loop failed");

        let sid = spawned.session_id.expect("pipe loop should yield session_id");
        assert!(spawned.output.contains("PIPE_STARTED"));

        // tty=false 时普通文本写入必须被 StdinClosed 拒绝。
        let write_err = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: "hello\n".to_string(),
                yield_time_ms: Some(250),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect_err("non-tty session must reject arbitrary stdin writes");
        assert!(
            matches!(write_err, ExecError::InvalidInput(ref msg) if msg.contains("tty=true")),
            "unexpected error: {write_err:?}"
        );

        // tty=false 时发送 "\u{0003}" 直接终止管道进程。
        let interrupted = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: "\u{0003}".to_string(),
                yield_time_ms: Some(3000),
                max_output_tokens: None,
            },
            None,
        )
        .await
        .expect("pipe Ctrl+C failed");

        assert!(
            interrupted.session_id.is_none(),
            "Ctrl+C on tty=false should terminate pipe process, got session_id={:?}, output={:?}",
            interrupted.session_id,
            interrupted.output
        );

        close_conversation(Some(&scope)).await;
    }

    #[tokio::test]
    async fn kill_process_keeps_exited_entry_in_listing_within_ttl() {
        let scope = unique_scope("kill-ttl");
        let cmd = if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 30".to_string()
        } else {
            "sleep 30".to_string()
        };

        let spawned = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope.clone()),
                cmd,
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: false,
                yield_time_ms: Some(1200),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("spawn failed");

        let sid = spawned.session_id.expect("should yield session_id");
        assert_eq!(list_processes_for_conversation(Some(&scope)).len(), 1);

        let killed = kill_process(Some(&scope), &sid.to_string())
            .await
            .expect("kill_process should succeed");
        assert!(!killed.running);
        assert!(killed.killed);

        // 手动 kill 后，在 5 分钟 TTL 内 UI 列表与快照仍应保留终止状态，避免刚停就报错 not found。
        let listed = list_processes_for_conversation(Some(&scope));
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].running);
        assert!(listed[0].killed);
        assert!(read_process_snapshot(Some(&scope), &sid.to_string(), Some(50)).is_ok());

        close_conversation(Some(&scope)).await;
    }

    #[tokio::test]
    async fn live_pty_snapshot_returns_output_while_running_and_after_kill() {
        let scope = unique_scope("live-pty-snap");
        let cmd = if cfg!(target_os = "windows") {
            "Write-Output 'nova-live-line-1'; Write-Output 'nova-live-line-2'; Start-Sleep -Seconds 30".to_string()
        } else {
            "echo 'nova-live-line-1'; echo 'nova-live-line-2'; sleep 30".to_string()
        };

        let spawned = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope.clone()),
                cmd,
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: true,
                yield_time_ms: Some(1200),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("spawn failed");

        let sid = spawned.session_id.expect("should yield session_id");

        // 进程仍在运行时，读取 UI 快照也必须能看到已产生的输出
        let mut snap = read_process_snapshot(Some(&scope), &sid.to_string(), Some(400))
            .expect("live snapshot failed");
        for _ in 0..30 {
            if snap.stdout.contains("nova-live-line-2") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            snap = read_process_snapshot(Some(&scope), &sid.to_string(), Some(400))
                .expect("live snapshot poll failed");
        }
        assert!(snap.running, "process should still be running");
        assert!(
            snap.stdout.contains("nova-live-line-1") && snap.stdout.contains("nova-live-line-2"),
            "expected live PTY stdout lines, got: {:?}",
            snap.stdout
        );

        let killed = kill_process(Some(&scope), &sid.to_string())
            .await
            .expect("kill_process should succeed");
        assert!(!killed.running);
        assert!(killed.killed);
        assert!(killed.finished);
        assert!(killed.stdout.contains("nova-live-line-2"));

        close_conversation(Some(&scope)).await;
    }

    #[tokio::test]
    async fn close_conversation_terminates_active_background_process() {
        let scope = unique_scope("close-kill");
        let cmd = if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 30".to_string()
        } else {
            "sleep 30".to_string()
        };

        let spawned = exec_command(
            ExecCommandRequest {
                conversation_id: Some(scope.clone()),
                cmd,
                workdir: test_workdir(),
                shell: None,
                login: false,
                tty: true,
                yield_time_ms: Some(300),
                max_output_tokens: None,
                unsandboxed_override: true,
                sandbox: None,
            },
            None,
        )
        .await
        .expect("spawn sleep failed");

        let sid = spawned.session_id.expect("sleep should yield session_id");
        assert_eq!(list_processes_for_conversation(Some(&scope)).len(), 1);

        close_conversation(Some(&scope)).await;
        assert!(list_processes_for_conversation(Some(&scope)).is_empty());

        let write_after_close = write_stdin(
            WriteStdinRequest {
                conversation_id: Some(scope.clone()),
                session_id: sid,
                chars: String::new(),
                yield_time_ms: Some(250),
                max_output_tokens: None,
            },
            None,
        )
        .await;
        assert!(write_after_close.is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_unified_exec_sandbox_pipe_and_conpty_work() {
        let root = tempfile::tempdir().expect("tempdir");
        let data_dir = root.path().join("data");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::create_dir_all(&outside).expect("outside");

        let sandbox = ExecSandboxConfig {
            policy: SandboxPolicy::WorkspaceWrite,
            workspace: workspace.clone(),
            data_dir,
        };

        for tty in [false, true] {
            let scope = format!("win_uexec_sandbox_{tty}_{}", NEXT_SESSION_ID.load(Ordering::Relaxed));
            let ok_file = workspace.join(format!("uexec_ok_{tty}.txt"));
            let blocked_file = outside.join(format!("uexec_blocked_{tty}.txt"));
            let cmd = format!(
                "Set-Content -Path '{}' -Value 'ok'; try {{ Set-Content -Path '{}' -Value 'no' -ErrorAction Stop }} catch {{ Write-Output 'BLOCKED_OUTSIDE' }}",
                ok_file.display(),
                blocked_file.display(),
            );
            let resp = exec_command(
                ExecCommandRequest {
                    conversation_id: Some(scope.clone()),
                    cmd,
                    workdir: workspace.clone(),
                    shell: None,
                    login: false,
                    tty,
                    yield_time_ms: Some(15_000),
                    max_output_tokens: None,
                    unsandboxed_override: false,
                    sandbox: Some(sandbox.clone()),
                },
                None,
            )
            .await
            .expect("sandboxed exec_command failed");

            assert!(resp.exit_code == Some(0), "tty={tty} exit={:?} out={}", resp.exit_code, resp.output);
            assert!(resp.output.contains("BLOCKED_OUTSIDE"), "tty={tty} out={}", resp.output);
            assert!(ok_file.exists(), "tty={tty} workspace file should exist");
            assert!(!blocked_file.exists(), "tty={tty} outside file must be blocked");
            close_conversation(Some(&scope)).await;
        }
    }
}
