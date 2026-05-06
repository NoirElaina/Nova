pub mod llm;
pub mod command;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 启动时自动创建 workspace 目录，确保 AI 有默认工作区。
            let ws = crate::llm::utils::system_prompt::workspace_dir(app.handle());
            if !ws.exists() {
                let _ = std::fs::create_dir_all(&ws);
            }

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::command::mcp::warmup_runtime(app_handle).await;
            });

            let scheduler_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::command::cron::run_scheduler_loop(scheduler_handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet, 
            llm::client::send_chat_message,
            llm::client::cancel_chat_message,
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
            command::history::load_history,
            command::history::append_history,
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
            command::skill::list_skills,
            command::skill::delete_skill,
            command::cron::list_scheduled_tasks,
            command::cron::create_scheduled_task,
            command::cron::delete_scheduled_task,
            command::settings::get_model_window_tokens,
            command::settings::estimate_text_tokens
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
