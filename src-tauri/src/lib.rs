pub mod command;
pub mod llm;
pub mod logging;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tracing::{info, warn};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    static SHELL_CLEANUP_ON_EXIT: OnceLock<AtomicBool> = OnceLock::new();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Err(error) = crate::logging::init(app.handle()) {
                eprintln!("[logging.init] {}", error);
            }

            // 初始化加密主密钥，必须在其他命令之前完成。
            if let Err(error) = crate::command::settings_secrets::init_master_key(app.handle()) {
                warn!(error = %error, "failed to initialize master encryption key");
            }

            info!("application setup started");

            // 启动时自动创建默认 workspace 目录，确保 AI 有默认工作区。
            match crate::command::workspace::default_workspace_root(app.handle()) {
                Ok(ws) => info!(path = %ws.display(), "default workspace directory ready"),
                Err(error) => {
                    warn!(error = %error, "failed to prepare default workspace directory")
                }
            }

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match crate::command::mcp::warmup_runtime(app_handle).await {
                    Ok(()) => info!("mcp runtime warmup completed"),
                    Err(error) => warn!(error = %error, "mcp runtime warmup failed"),
                }
            });

            let scheduler_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                info!("scheduler loop starting");
                crate::command::cron::run_scheduler_loop(scheduler_handle).await;
            });
            info!("application setup completed");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            llm::client::send_chat_message,
            llm::client::cancel_chat_message,
            llm::client::get_chat_turn_status,
            llm::client::ack_chat_turn_status,
            llm::client::submit_permission_decision,
            command::settings::get_settings,
            command::settings::save_settings,
            command::agent_config::list_agent_profiles,
            command::agent_config::create_agent_profile,
            command::agent_config::delete_agent_profile,
            command::agent_config::load_agent_profile_markdown,
            command::agent_config::save_agent_profile_markdown,
            command::agent_config::get_agent_markdown_path,
            command::agent_config::load_agent_markdown,
            command::agent_config::save_agent_markdown,
            command::history::create_conversation,
            command::history::list_conversations,
            command::history::set_conversation_pinned,
            command::history::export_conversation,
            command::history::export_rendered_conversation_pdf,
            command::history::load_history,
            command::history::append_history,
            command::history::replace_history,
            command::history::load_conversation_tool_logs,
            command::history::upsert_conversation_tool_log,
            command::history::clear_history,
            command::history::delete_conversation,
            command::history::get_conversation_memory,
            command::history::get_conversation_handover,
            command::history::get_conversation_compact_context,
            command::history::get_latest_compact_boundary,
            command::history::get_conversation_resume_context,
            command::history::upsert_conversation_memory,
            command::history::list_global_memory,
            command::history::upsert_global_memory,
            command::history::delete_global_memory,
            command::history::clear_global_memory,
            command::file_changes::list_file_changes,
            command::file_changes::get_file_change,
            command::file_changes::revert_file_change,
            command::file_changes::init_git_repo,
            command::file_changes::get_git_repo_status,
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
            command::rag::rag_get_stats,
            command::rag::rag_list_documents,
            command::rag::rag_list_conversation_documents,
            command::rag::rag_read_document,
            command::rag::rag_upsert_documents,
            command::rag::rag_upsert_conversation_documents,
            command::rag::rag_remove_document,
            command::rag::rag_clear_documents,
            command::shell::get_shell_session_status,
            command::shell::execute_shell_command_for_conversation,
            command::user_terminal::user_terminal_start,
            command::user_terminal::user_terminal_write,
            command::user_terminal::user_terminal_resize,
            command::user_terminal::user_terminal_stop,
            command::skill::list_skills,
            command::skill::delete_skill,
            command::cron::list_scheduled_tasks,
            command::cron::create_scheduled_task,
            command::cron::delete_scheduled_task,
            command::settings::get_model_window_tokens,
            command::settings::estimate_text_tokens,
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
            command::workspace::get_workspace_root,
            command::workspace::set_default_workspace_root,
            command::workspace::workspace_list_directory,
            command::workspace::workspace_read_text_file,
            command::workspace::get_workspace_context,
            command::usage::get_usage_stats,
            command::usage::list_token_usage,
            command::pet_config::fetch_pet,
            command::pet_config::download_pet,
            command::pet_config::list_local_pets,
            command::pet_config::delete_local_pet,
            command::pet_config::get_pet_spritesheet,
            command::pet_config::launch_desktop_pet,
            command::pet_config::close_desktop_pet,
            command::pet_config::get_pet_window_config
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let already_cleaned = SHELL_CLEANUP_ON_EXIT
                .get_or_init(|| AtomicBool::new(false))
                .swap(true, Ordering::Relaxed);
            if !already_cleaned {
                tauri::async_runtime::block_on(
                    crate::llm::services::shell_sessions::close_all_sessions(),
                );
                crate::llm::services::user_terminal::close_all_sessions();
            }
        }
    });
}
