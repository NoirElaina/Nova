pub mod command;
pub mod daemon;
pub mod logging;
pub mod services;
pub mod supervisor;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tracing::info;

/// Nova 桌面薄壳：窗口 + daemon 客户端 + 桌面能力服务端。
/// Agent 内核完全由 nova-cli 守护进程承载，本进程不含任何内核实现。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    static TERMINAL_CLEANUP_ON_EXIT: OnceLock<AtomicBool> = OnceLock::new();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Err(error) = crate::logging::init(app.handle()) {
                eprintln!("[logging.init] {}", error);
            }

            info!("desktop shell setup started");

            // 部署 CLI 并以其 stdio 管道拉起内核子进程（不监听端口）。
            let cli_path = crate::supervisor::init_desktop_supervisor(app.handle());
            crate::daemon::init(app.handle(), cli_path);

            info!("desktop shell setup completed");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            command::settings::get_available_terminals,
            command::agent::send_modern_agent_turn,
            command::agent::cancel_modern_agent_turn,
            command::agent::send_branch_message,
            command::agent::cancel_branch_message,
            command::permission_rules::submit_permission_decision,
            command::settings::get_settings,
            command::settings::save_settings,
            command::agent_config::list_agent_bundles,
            command::agent_config::create_agent_bundle,
            command::agent_config::load_agent_bundle,
            command::agent_config::save_agent_bundle,
            command::agent_config::delete_agent_bundle,
            command::agent_config::get_conversation_agent,
            command::agent_config::set_conversation_agent,
            command::agent_config::list_configurable_tools,
            command::agent_config::list_agent_private_skills,
            command::agent_config::list_agent_files,
            command::agent_config::import_agent_file,
            command::agent_config::delete_agent_file,
            command::agent_config::reveal_agent_dir,
            command::get_app_data_dir,
            command::history::create_conversation,
            command::history::list_conversations,
            command::history::set_conversation_pinned,
            command::history::export_conversation,
            command::history::export_rendered_conversation_pdf,
            command::history::load_history,
            command::history::replace_history,
            command::history::append_plain_chat_message,
            command::history::load_conversation_tool_logs,
            command::history::get_conversation_turn_traces,
            command::history::clear_history,
            command::history::delete_conversation,
            command::history::list_memory_entries,
            command::history::add_memory_entry,
            command::history::remove_memory_entry,
            command::history::clear_memory_entries,
            command::history::manual_compact_conversation,
            command::file_changes::get_workspace_diff,
            command::file_changes::init_git_repo,
            command::file_changes::get_git_repo_status,
            command::file_changes::get_workspace_git_status,
            command::mcp::add_mcp_server,
            command::mcp::get_mcp_server,
            command::mcp::update_mcp_server,
            command::mcp::remove_mcp_server,
            command::mcp::get_mcp_server_statuses,
            command::mcp::reload_all_mcp_servers,
            command::mcp::set_mcp_server_enabled,
            command::mcp::list_mcp_tools,
            command::mcp::list_mcp_resources,
            command::mcp::read_mcp_resource,
            command::mcp::call_mcp_tool,
            command::session_files::save_session_file,
            command::session_files::list_session_files,
            command::session_files::read_session_file,
            command::shell::get_shell_session_status,
            command::shell::list_background_jobs,
            command::shell::read_background_job_output,
            command::user_terminal::user_terminal_start,
            command::user_terminal::user_terminal_write,
            command::user_terminal::user_terminal_resize,
            command::user_terminal::user_terminal_stop,
            command::skill::list_skills,
            command::skill::read_skill_content,
            command::skill::delete_skill,
            command::cron::list_scheduled_tasks,
            command::cron::create_scheduled_task,
            command::cron::delete_scheduled_task,
            command::settings::get_model_window_tokens,
            command::settings::estimate_text_tokens,
            command::settings::refresh_model_catalog,
            command::settings::get_model_catalog_stats,
            command::settings::get_model_catalog,
            command::model_fetch::fetch_available_models,
            command::browser::browser_navigate_window,
            command::browser::browser_reload_window,
            command::browser::browser_eval_window_script,
            command::browser::browser_eval_window_script_result,
            command::browser::browser_call_devtools_protocol_method,
            command::browser::register_browser_session,
            command::browser::unregister_browser_session,
            command::browser::update_browser_session_url,
            command::browser::browser_automation_result,
            command::browser::load_browser_tab_state,
            command::browser::save_browser_tab_state,
            command::browser::clear_browser_tab_state,
            command::workspace::workspace_list_directory,
            command::workspace::workspace_read_text_file,
            command::usage::get_usage_stats,
            command::usage::list_token_usage,
            command::usage::get_conversation_usage,
            command::todo::list_todos,
            command::plan::get_conversation_plan,
            command::hooks::get_hooks_toml,
            command::hooks::get_hooks_structured,
            command::hooks::save_hooks_toml,
            command::permission_rules::list_permission_rules,
            command::permission_rules::delete_permission_rule
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            // 显式释放客户端：drop Client 会触发 kill_on_drop，结束内核子进程。
            crate::daemon::shutdown();
            let already_cleaned = TERMINAL_CLEANUP_ON_EXIT
                .get_or_init(|| AtomicBool::new(false))
                .swap(true, Ordering::Relaxed);
            if !already_cleaned {
                crate::services::user_terminal::close_all_sessions();
            }
        }
    });
}
