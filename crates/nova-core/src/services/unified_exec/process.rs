//! 托管交互进程实例：输出环形缓冲区、模型增量游标、UI 只读快照与进程树终止。

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use nova_protocol::misc::BackgroundOutput;
use portable_pty::{ChildKiller, MasterPty};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{watch, Mutex as AsyncMutex, Notify};

#[cfg(windows)]
use crate::agent::sandbox::windows::{WinConPty, WinJobObject};

use super::vt_clean::collapse_line_controls;

/// 初次 `exec_command` 默认等待输出时间（10s）。
pub const DEFAULT_YIELD_TIME_MS: u64 = 10_000;
/// `write_stdin` 默认等待输出时间（250ms）。
pub const DEFAULT_WRITE_STDIN_YIELD_TIME_MS: u64 = 250;
/// 允许的最小/最大单次让出等待窗口。
pub const MIN_YIELD_TIME_MS: u64 = 250;
pub const MAX_YIELD_TIME_MS: u64 = 30_000;
/// 模型工具返回默认的输出 token 上限。
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
/// 每个进程保留的滚动输出上限（1 MiB）。
pub const UNIFIED_EXEC_OUTPUT_MAX_BYTES: usize = 1024 * 1024;
/// 全局最多托管的活跃/最近完成进程上限（LRU 淘汰已退出进程）。
pub const MAX_UNIFIED_EXEC_PROCESSES: usize = 64;
/// 子进程退出后等待读取线程排干尾部缓冲的最大时间。
pub const EXIT_DRAIN_MS: u64 = 80;
/// Windows 上 `exec_command` 的最小让出窗口：ConPTY 冷启动本身就要数百毫秒，
/// 低于 10s 的 yield 会让几乎所有命令先 yield 一次再被 poll（对齐 Codex）。
pub const WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS: u64 = 10_000;
/// 空 `write_stdin`（纯轮询）的最小等待：避免 250ms 级轮询风暴（对齐 Codex）。
pub const MIN_EMPTY_YIELD_TIME_MS: u64 = 5_000;
/// 空 `write_stdin` 的最大等待：允许一次 poll 覆盖长构建（对齐 Codex 的 5 分钟）。
pub const MAX_EMPTY_YIELD_TIME_MS: u64 = 300_000;
/// 已完成会话在"后台任务"列表中的保留时长，超时后不再展示（防止陈旧条目堆积）。
pub const COMPLETED_JOB_TTL_MS: u64 = 5 * 60 * 1000;

pub type SessionId = i32;

#[derive(Debug, Clone, Copy)]
pub struct ExitStatusSnapshot {
    pub exit_code: Option<i32>,
    pub killed: bool,
    pub timed_out: bool,
    pub finished_at: Instant,
}

/// 头尾双段有界输出缓冲（对齐 Codex `HeadTailBuffer`）。
///
/// 为什么不是纯尾部环形缓冲：编译/测试日志的**第一条错误**往往在最前面，
/// 后续再刷几 MB 警告时，纯尾环会把开头整段挤掉，模型永远看不到真正的失败原因。
/// 这里把 1 MiB 预算对半分：前 512 KiB 永久保留，后 512 KiB 滚动保留，
/// 中间被丢弃的字节数单独记账，读取时用显式 marker 标出。
#[derive(Debug)]
pub struct RollingOutputBuffer {
    /// 稳定前缀（最多 `UNIFIED_EXEC_OUTPUT_MAX_BYTES / 2`）。
    head: Vec<u8>,
    /// 一旦开始溢入 `tail`（哪怕因 UTF-8 字符边界导致 `head` 还差 1~3 字节），立即封口避免后续 ASCII 倒序插队。
    head_sealed: bool,
    /// 最新后缀（最多剩余预算）。
    tail: VecDeque<u8>,
    /// 被丢弃的中间字节数（用于 marker 与 dropped 统计）。
    omitted: u64,
    /// 历史累计写入字节数，作为模型游标的绝对坐标系。
    total_written: u64,
    truncated_ever: bool,
}

impl Default for RollingOutputBuffer {
    fn default() -> Self {
        Self {
            head: Vec::with_capacity(16 * 1024),
            head_sealed: false,
            tail: VecDeque::with_capacity(16 * 1024),
            omitted: 0,
            total_written: 0,
            truncated_ever: false,
        }
    }
}

impl RollingOutputBuffer {
    const HEAD_BUDGET: usize = UNIFIED_EXEC_OUTPUT_MAX_BYTES / 2;
    const TAIL_BUDGET: usize = UNIFIED_EXEC_OUTPUT_MAX_BYTES - Self::HEAD_BUDGET;

    pub fn push_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut incoming = text.as_bytes();
        self.total_written += incoming.len() as u64;

        // 1. 先填稳定前缀；切分点必须落在 UTF-8 字符边界上；一旦开始溢入 tail 即封口不再插入 head。
        if !self.head_sealed {
            let head_room = Self::HEAD_BUDGET.saturating_sub(self.head.len());
            if head_room > 0 {
                let mut take = head_room.min(incoming.len());
                // `head` 末尾始终是字符边界，新片段只需按自身边界裁剪。
                while take > 0 && !text.is_char_boundary(take) {
                    take -= 1;
                }
                if take > 0 {
                    self.head.extend_from_slice(&incoming[..take]);
                    incoming = &incoming[take..];
                }
            }
            if incoming.is_empty() {
                return;
            }
            self.head_sealed = true;
        }

        // 2. 其余进滚动后缀；超预算时从队头丢弃并记账。
        self.tail.extend(incoming.iter().copied());
        if self.tail.len() > Self::TAIL_BUDGET {
            let mut drop_count = self.tail.len() - Self::TAIL_BUDGET;
            // 前向对齐到合法 UTF-8 首字节（跳过 continuation byte `10xxxxxx`）。
            while drop_count < self.tail.len() && (self.tail[drop_count] & 0xC0) == 0x80 {
                drop_count += 1;
            }
            self.tail.drain(..drop_count);
            self.omitted += drop_count as u64;
            self.truncated_ever = true;
        }
    }

    pub fn total_written(&self) -> u64 {
        self.total_written
    }

    fn format_omission_marker(omitted: u64) -> String {
        format!("\n... {omitted} bytes omitted ...\n")
    }

    fn marker(&self) -> String {
        Self::format_omission_marker(self.omitted)
    }

    /// Copy contiguous `[skip..]` bytes from `tail` before decoding UTF-8 so a character
    /// spanning `VecDeque`'s ring wrap boundary does not turn into `U+FFFD`.
    fn append_tail_utf8(&self, dest: &mut String, skip: usize) {
        if skip >= self.tail.len() {
            return;
        }
        let (front, back) = self.tail.as_slices();
        let mut out: Vec<u8> = Vec::with_capacity(self.tail.len() - skip);
        if skip < front.len() {
            out.extend_from_slice(&front[skip..]);
            out.extend_from_slice(back);
        } else {
            let back_start = skip - front.len();
            if back_start < back.len() {
                out.extend_from_slice(&back[back_start..]);
            }
        }
        dest.push_str(&String::from_utf8_lossy(&out));
    }

    /// 读取 `[cursor..total_written]` 区间内容，返回 `(text, new_cursor, dropped_bytes)`。
    /// 本方法本身只读；`model_cursor` 由调用方在返回给模型前推进。
    ///
    /// `text` 在跨越被丢弃的中间段时会自带 `... N bytes omitted ...` 标记，
    /// 并折叠终端 `\r` / 退格 / 擦行控制符以保证增量输出整洁。
    pub fn read_since(&self, cursor: u64) -> (String, u64, u64) {
        if cursor >= self.total_written {
            return (String::new(), self.total_written, 0);
        }

        let head_len = self.head.len() as u64;
        let tail_start = head_len + self.omitted;

        // 场景 (a)：起点落在稳定前缀内：输出 前缀剩余段 + (如有丢弃则插 marker) + 整个后缀。
        if cursor < head_len {
            let mut text = String::from_utf8_lossy(&self.head[cursor as usize..]).into_owned();
            if self.omitted > 0 {
                text.push_str(&self.marker());
            }
            self.append_tail_utf8(&mut text, 0);
            return (
                collapse_line_controls(&text),
                self.total_written,
                self.omitted,
            );
        }

        // 场景 (b)：起点落在已被滚动丢弃的中间段（head_len <= cursor < tail_start）：
        // 未读丢失字节数 = tail_start - cursor；在非空后缀前插入精确 marker，避免静默丢数。
        if cursor < tail_start {
            let dropped = tail_start.saturating_sub(cursor);
            let mut text = String::new();
            if !self.tail.is_empty() && dropped > 0 {
                text.push_str(&Self::format_omission_marker(dropped));
                self.append_tail_utf8(&mut text, 0);
            }
            return (collapse_line_controls(&text), self.total_written, dropped);
        }

        // 场景 (c)：起点落在可读后缀内（cursor >= tail_start）：零未读丢失，直接切片返回。
        let skip = (cursor - tail_start) as usize;
        if skip >= self.tail.len() {
            return (String::new(), self.total_written, 0);
        }
        let mut text = String::new();
        self.append_tail_utf8(&mut text, skip);
        (collapse_line_controls(&text), self.total_written, 0)
    }

    /// UI 面板专用只读快照：返回最近 `tail_lines` 行，绝不影响模型游标。
    pub fn snapshot_tail(&self, tail_lines: usize) -> (String, bool) {
        let mut raw: Vec<u8> = Vec::with_capacity(self.head.len() + self.tail.len() + 32);
        raw.extend_from_slice(&self.head);
        if self.omitted > 0 {
            raw.extend_from_slice(self.marker().as_bytes());
        }
        let (front, back) = self.tail.as_slices();
        raw.extend_from_slice(front);
        raw.extend_from_slice(back);
        let raw_text = String::from_utf8_lossy(&raw).into_owned();
        let cleaned = collapse_line_controls(&raw_text);
        if tail_lines == 0 {
            return (cleaned, self.truncated_ever);
        }

        let lines: Vec<&str> = cleaned.lines().collect();
        if lines.len() <= tail_lines {
            return (cleaned, self.truncated_ever);
        }
        let tail = lines[lines.len() - tail_lines..].join("\n");
        (tail, true)
    }
}

pub enum StdinSink {
    Pty(Arc<StdMutex<Option<Box<dyn Write + Send>>>>),
    Pipe(Arc<AsyncMutex<Option<Box<dyn AsyncWrite + Send + Unpin>>>>),
}

impl StdinSink {
    pub async fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        match self {
            StdinSink::Pty(writer) => {
                let writer = Arc::clone(writer);
                let data = bytes.to_vec();
                tokio::task::spawn_blocking(move || {
                    let mut guard = writer.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(ref mut w) = *guard else {
                        return Err("Process stdin/PTY is closed".to_string());
                    };
                    w.write_all(&data)
                        .and_then(|()| w.flush())
                        .map_err(|e| format!("Failed to write to PTY stdin: {e}"))
                })
                .await
                .map_err(|e| format!("PTY write task failed: {e}"))?
            }
            StdinSink::Pipe(stdin) => {
                let mut guard = stdin.lock().await;
                let Some(ref mut child_stdin) = *guard else {
                    return Err("Process stdin is closed".to_string());
                };
                child_stdin
                    .write_all(bytes)
                    .await
                    .map_err(|e| format!("Failed to write to pipe stdin: {e}"))?;
                child_stdin
                    .flush()
                    .await
                    .map_err(|e| format!("Failed to flush pipe stdin: {e}"))?;
                Ok(())
            }
        }
    }

    pub async fn close(&self) {
        match self {
            StdinSink::Pty(writer) => {
                let mut guard = writer.lock().unwrap_or_else(|e| e.into_inner());
                let _ = guard.take();
            }
            StdinSink::Pipe(stdin) => {
                let mut guard = stdin.lock().await;
                let _ = guard.take();
            }
        }
    }
}

pub struct PtyControl {
    pub killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    pub master: Option<Box<dyn MasterPty + Send>>,
    #[cfg(windows)]
    pub win_conpty: Option<WinConPty>,
}

impl PtyControl {
    pub fn kill_and_close(&mut self) {
        if let Some(ref mut killer) = self.killer {
            let _ = killer.kill();
        }
        self.killer = None;
        // 显式 drop MasterPty / WinConPty：关闭 ConPTY / pty master 句柄，确保读取线程尽快收到 EOF 退出。
        self.master = None;
        #[cfg(windows)]
        {
            self.win_conpty = None;
        }
    }
}

pub struct ManagedProcess {
    pub session_id: SessionId,
    pub scope: String,
    pub pid: u32,
    pub command: String,
    pub workdir: String,
    pub shell: String,
    pub tty: bool,
    pub started_at: Instant,
    pub last_activity_at: StdMutex<Instant>,
    /// 仅当命令超过首次 `yield_time_ms` 转入后台持久交互，才在 UI "后台任务"芯片与抽屉中展示。
    pub yielded_to_background: AtomicBool,
    pub buffer: Arc<StdMutex<RollingOutputBuffer>>,
    pub model_cursor: StdMutex<u64>,
    pub stdin: Arc<StdinSink>,
    /// 串行化同一会话上的 `write_stdin` 写-等交互，对齐 Codex `interaction_lock`。
    pub interaction_lock: Arc<AsyncMutex<()>>,
    pub output_notify: Arc<Notify>,
    pub exit_tx: watch::Sender<Option<ExitStatusSnapshot>>,
    pub exit_rx: watch::Receiver<Option<ExitStatusSnapshot>>,
    pub reader_done_rx: watch::Receiver<bool>,
    pub pty_control: StdMutex<Option<PtyControl>>,
    #[cfg(windows)]
    pub win_job: StdMutex<Option<WinJobObject>>,
    pub killed_flag: AtomicBool,
}

impl ManagedProcess {
    pub fn has_exited(&self) -> bool {
        self.exit_rx.borrow().is_some()
    }

    pub fn exit_snapshot(&self) -> Option<ExitStatusSnapshot> {
        *self.exit_rx.borrow()
    }

    /// 会话结束后经过的毫秒数；仍在运行时返回 `None`。
    pub fn finished_age_ms(&self) -> Option<u64> {
        self.exit_snapshot()
            .map(|s| s.finished_at.elapsed().as_millis() as u64)
    }

    pub fn touch_activity(&self) {
        if let Ok(mut guard) = self.last_activity_at.lock() {
            *guard = Instant::now();
        }
    }

    pub fn last_activity(&self) -> Instant {
        self.last_activity_at
            .lock()
            .map(|g| *g)
            .unwrap_or(self.started_at)
    }

    /// 获取自上次模型读取之后的增量文本，并向前推进 `model_cursor`。
    /// 跨越被丢弃的中间段时，文本内部已带 `... N bytes omitted ...` 标记。
    pub fn drain_model_delta(&self) -> String {
        let mut cursor = self.model_cursor.lock().unwrap_or_else(|e| e.into_inner());
        let buf = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let (delta, next_cursor, _dropped) = buf.read_since(*cursor);
        *cursor = next_cursor;
        delta
    }

    pub fn total_buffered_written(&self) -> u64 {
        self.buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total_written()
    }

    /// 彻底终止进程树并关闭 PTY/管道句柄。
    pub async fn kill_tree(&self) {
        self.killed_flag.store(true, Ordering::SeqCst);
        self.stdin.close().await;

        #[cfg(windows)]
        let win_job = {
            let mut guard = self.win_job.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };

        let pid = self.pid;
        if pid > 0 {
            let _ = tokio::task::spawn_blocking(move || {
                #[cfg(windows)]
                if let Some(job) = win_job {
                    job.terminate();
                    drop(job);
                }
                kill_os_process_tree(pid);
            })
            .await;
        } else {
            #[cfg(windows)]
            if let Some(job) = win_job {
                let _ = tokio::task::spawn_blocking(move || {
                    job.terminate();
                })
                .await;
            }
        }

        // 先尝试关闭 PTY killer 与 master 句柄（同样放到 blocking 防止关闭 ConPTY 轻微卡顿事件循环）
        let pty_ctrl = {
            let mut guard = self.pty_control.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        if let Some(mut ctrl) = pty_ctrl {
            let _ = tokio::task::spawn_blocking(move || {
                ctrl.kill_and_close();
            })
            .await;
        }

        // 若 OS waiter 线程尚未来得及上报（或句柄已强关），确保 watch 状态写入 killed=true
        let already_exited = self.exit_rx.borrow().is_some();
        if !already_exited {
            let _ = self.exit_tx.send(Some(ExitStatusSnapshot {
                exit_code: None,
                killed: true,
                timed_out: false,
                finished_at: Instant::now(),
            }));
        }
        self.output_notify.notify_waiters();
    }

    /// 供前端 UI（芯片与任务抽屉）只读查看；不改变模型游标。
    pub fn to_background_output(&self, include_output: bool, tail_lines: usize) -> BackgroundOutput {
        let exit = self.exit_snapshot();
        let running = exit.is_none();
        let finished = exit.is_some();
        let exit_code = exit.and_then(|s| s.exit_code);
        let killed = exit.map(|s| s.killed).unwrap_or_else(|| self.killed_flag.load(Ordering::SeqCst));
        let timed_out = exit.map(|s| s.timed_out).unwrap_or(false);
        let elapsed_ms = exit
            .map(|s| s.finished_at.saturating_duration_since(self.started_at))
            .unwrap_or_else(|| self.started_at.elapsed())
            .as_millis() as u64;

        let (stdout, truncated) = if include_output {
            let buf = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
            buf.snapshot_tail(tail_lines)
        } else {
            (String::new(), false)
        };

        BackgroundOutput {
            id: self.session_id.to_string(),
            pid: self.pid,
            command: self.command.clone(),
            cwd: Some(crate::services::workspace::display_path_text(&self.workdir)),
            running,
            finished,
            exit_code,
            killed,
            timed_out,
            stdout,
            stderr: String::new(),
            truncated,
            elapsed_ms,
            ttl_ms: 0,
            remaining_ms: 0,
        }
    }
}

/// 跨平台杀死整棵进程树（不留孤儿节点/Python/Cargo/Vite 子进程）。
pub fn kill_os_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Stdio;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status();
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::time::Duration;
        let pgid = pid as libc::pid_t;
        unsafe {
            let _ = libc::kill(-pgid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(40));
        unsafe {
            if libc::kill(-pgid, 0) == 0 {
                let _ = libc::kill(-pgid, libc::SIGKILL);
            }
            let _ = libc::kill(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_buffer_keeps_head_and_tail_on_utf8_boundaries() {
        let mut buf = RollingOutputBuffer::default();
        // 插入略多于 1 MiB 的多字节中文文本
        let chunk = "中".repeat(4096); // 12 KiB
        for _ in 0..95 {
            buf.push_str(&chunk);
        }
        assert!(buf.total_written() > UNIFIED_EXEC_OUTPUT_MAX_BYTES as u64);

        let (text, next_cursor, dropped) = buf.read_since(0);
        assert!(dropped > 0);
        assert_eq!(next_cursor, buf.total_written());
        assert!(text.contains("bytes omitted"), "must mark the dropped middle");
        // 去掉 marker 后，保留部分必须全是完整的中文字符（没有切半的 U+FFFD）。
        let body: String = text
            .lines()
            .filter(|line| !line.contains("bytes omitted"))
            .collect();
        assert!(body.chars().all(|c| c == '中' || c == '\n'));
        assert!(!body.contains('\u{FFFD}'));
        assert!(buf.head.len() + buf.tail.len() <= UNIFIED_EXEC_OUTPUT_MAX_BYTES);
    }

    #[test]
    fn rolling_buffer_read_since_spans_head_marker_tail() {
        let mut buf = RollingOutputBuffer::default();
        buf.push_str("EARLY-ERROR\n");
        let big = "x".repeat(UNIFIED_EXEC_OUTPUT_MAX_BYTES);
        buf.push_str(&big);
        buf.push_str("TAIL-LINE\n");

        // 模型从头读：必须同时看到最早的错误与最新的尾行。
        let (text, cursor, dropped) = buf.read_since(0);
        assert!(text.starts_with("EARLY-ERROR"));
        assert!(text.ends_with("TAIL-LINE\n"));
        assert!(dropped > 0);

        // 增量读：游标已到末尾，再读应为空。
        let (again, cursor2, dropped2) = buf.read_since(cursor);
        assert!(again.is_empty());
        assert_eq!(cursor2, cursor);
        assert_eq!(dropped2, 0);
    }

    #[test]
    fn read_since_tracks_incremental_cursor_independently_of_ui_snapshot() {
        let mut buf = RollingOutputBuffer::default();
        buf.push_str("line1\n");
        let (first, c1, dropped1) = buf.read_since(0);
        assert_eq!(first, "line1\n");
        assert_eq!(dropped1, 0);

        // UI 读取全部快照绝不改变 c1
        let (snap, _) = buf.snapshot_tail(50);
        assert_eq!(snap, "line1\n");

        buf.push_str("line2\n");
        let (second, c2, _) = buf.read_since(c1);
        assert_eq!(second, "line2\n");
        assert!(c2 > c1);
    }

    #[test]
    fn read_since_handles_dropped_middle_and_tail_incrementally() {
        let mut buf = RollingOutputBuffer::default();
        // 先写满 head + 100B 进入 tail，读取后游标 c1 落在 head_len + 100。
        let initial_len = RollingOutputBuffer::HEAD_BUDGET + 100;
        buf.push_str(&"H".repeat(RollingOutputBuffer::HEAD_BUDGET));
        buf.push_str(&"A".repeat(100));
        let (_first, c1, dropped1) = buf.read_since(0);
        assert_eq!(dropped1, 0);
        assert_eq!(c1, initial_len as u64);

        // 再写入远超 TAIL_BUDGET 的输出，把原本 A*100 连同新增中间段一起挤掉（场景 b）。
        let flood = RollingOutputBuffer::TAIL_BUDGET + 2048;
        buf.push_str(&"B".repeat(flood));
        buf.push_str("END-MARKER\n");

        let (mid_text, c2, dropped2) = buf.read_since(c1);
        let expected_tail_start = buf.total_written() - (RollingOutputBuffer::TAIL_BUDGET as u64);
        // Since TAIL_BUDGET is trimmed at UTF-8 boundaries and ASCII is 1 byte, tail_start matches exact excess.
        let expected_dropped = expected_tail_start - c1;
        assert_eq!(dropped2, expected_dropped);
        assert!(dropped2 > 0);
        assert!(
            mid_text.starts_with(&format!("\n... {expected_dropped} bytes omitted ...\n")),
            "scenario (b) must prepend exact unread dropped count marker, got head={:?}",
            mid_text.lines().take(3).collect::<Vec<_>>()
        );
        assert!(mid_text.ends_with("END-MARKER\n"));

        // 场景 (c)：此时再追加少量输出，cursor=c2 >= tail_start，应零丢失且不附带 marker。
        buf.push_str("AFTER\n");
        let (tail_text, c3, dropped3) = buf.read_since(c2);
        assert_eq!(dropped3, 0);
        assert_eq!(tail_text, "AFTER\n");
        assert_eq!(c3, buf.total_written());
    }

    #[test]
    fn head_sealed_prevents_ascii_jumping_ahead_after_utf8_overflow() {
        let mut buf = RollingOutputBuffer::default();
        let almost_full = RollingOutputBuffer::HEAD_BUDGET - 2;
        buf.push_str(&"a".repeat(almost_full));
        // '中' 占 3 字节，无法放入剩余 2 字节的 head -> 整体溢入 tail，并立即封死 head。
        buf.push_str("中X");
        // 后续 1 字节 ASCII 不能再插队进入 head 剩余的 2 字节空隙。
        buf.push_str("Z");

        assert_eq!(buf.head.len(), almost_full);
        let (text, _, dropped) = buf.read_since(0);
        assert_eq!(dropped, 0);
        assert!(text.ends_with("中XZ"), "got tail {:?}", &text[text.len().saturating_sub(16)..]);
    }

    #[test]
    fn read_since_collapses_carriage_return_progress_lines() {
        let mut buf = RollingOutputBuffer::default();
        buf.push_str("progress 10%\rprogress 50%\rprogress 100%\n");
        let (text, _, _) = buf.read_since(0);
        assert_eq!(text, "progress 100%\n");
    }
}
