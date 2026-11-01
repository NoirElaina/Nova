//! 领域应用服务层（单一事实源）：CLI / 守护进程 / 桌面端共用的全部应用操作。
//!
//! - 桌面端（过渡态）：src-tauri 的 #[tauri::command] shim 逐名转发到这里；
//! - 守护进程：JSON-RPC 分发表按命令名调用这里；
//! - CLI：直接调用。
//!
//! 所有函数只依赖 Core，不引用任何宿主类型。

pub mod agent_config;
pub mod cron;
pub mod file_changes;
pub mod history;
pub mod hooks;
pub mod mcp;
pub mod model_fetch;
pub mod permission_rules;
pub mod plan;
pub mod session_files;
pub mod settings;
pub mod settings_secrets;
pub mod shell;
pub mod skill;
pub mod todo;
pub mod usage;
pub mod workspace;
