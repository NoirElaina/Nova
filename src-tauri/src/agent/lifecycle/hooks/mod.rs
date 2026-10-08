mod command;
mod config;
mod dispatch;
mod types;

pub(crate) use config::{hooks_file_path, validate_hooks_toml};
pub use config::{invalidate_hooks_cache, HooksFile};
pub use dispatch::{
    emit_hook_event, run_post_tool_use_failure_hooks, run_post_tool_use_hooks,
    run_pre_tool_use_hooks, run_subagent_start_hooks, run_subagent_stop_hooks,
};
pub use types::{HookEvent, HookOutcome};

/// 当前 hooks.toml 配置的处理器总数（配置摘要展示用）。
pub fn config_handler_count(app: &tauri::AppHandle) -> usize {
    config::load_hooks_file(app).hooks.handler_count()
}
