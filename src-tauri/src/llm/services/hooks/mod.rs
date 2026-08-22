mod command;
mod config;
mod dispatch;
mod shared;
mod types;

pub(crate) use config::{hooks_file_path, validate_hooks_toml};
pub use dispatch::{
    run_error_hooks, run_post_compact_hooks, run_post_tool_use_failure_hooks,
    run_post_tool_use_hooks, run_pre_compact_hooks, run_pre_tool_use_hooks,
    run_session_end_hooks, run_session_start_hooks, run_stop_hooks, run_subagent_start_hooks,
    run_subagent_stop_hooks, run_user_prompt_submit_hooks,
};
pub use types::HookOutcome;

/// 当前 hooks.toml 配置的处理器总数（配置摘要展示用）。
pub fn config_handler_count(app: &tauri::AppHandle) -> usize {
    config::load_hooks_file(app).hooks.handler_count()
}
