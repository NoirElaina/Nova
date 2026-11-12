//! Codex `unified_exec` 交互式终端与持久化进程执行子系统。

pub mod manager;
pub mod process;
pub mod vt_clean;

pub use manager::{
    close_all, close_conversation, exec_command, kill_process, list_processes_for_conversation,
    read_process_snapshot, write_stdin, ExecCommandRequest, ExecError, ExecSandboxConfig,
    ExecToolResponse, WriteStdinRequest,
};
pub use process::{
    SessionId, DEFAULT_MAX_OUTPUT_TOKENS, DEFAULT_WRITE_STDIN_YIELD_TIME_MS,
    DEFAULT_YIELD_TIME_MS, MAX_EMPTY_YIELD_TIME_MS, MAX_UNIFIED_EXEC_PROCESSES,
    MAX_YIELD_TIME_MS, MIN_EMPTY_YIELD_TIME_MS, MIN_YIELD_TIME_MS,
    UNIFIED_EXEC_OUTPUT_MAX_BYTES, WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS,
};
