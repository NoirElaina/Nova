//! 唯一 RPC 命令分发器：方法名与前端 IPC 命令一一对应。
//!
//! 本文件由 api 签名机械生成，禁止手工维护业务分支——
//! 新增/变更应用操作时只改 nova_core::api，重新生成即可。

use nova_core::Core;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

/// 分发错误：JSON-RPC 标准错误码 + 人类可读信息。
///
/// - `-32601` 方法不存在
/// - `-32602` 参数非法
/// - `-32603` 内核内部错误
pub type DispatchError = (i32, String);

pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

pub async fn dispatch(
    core: &Core,
    method: &str,
    params: Value,
    client_id: Option<&str>,
) -> Result<Value, DispatchError> {
    match method {
        // 长任务：在 tokio::spawn 之前同步完成并发/Steer/Queue 仲裁并返回确定性回包。
        "send_modern_agent_turn" => {
            let conversation_id: String = arg(&params, "conversationId")?;
            let prompt: String = arg(&params, "prompt")?;
            let attachments = arg(&params, "attachments")?;
            let mode: nova_protocol::agent::TurnSubmitMode =
                arg::<Option<nova_protocol::agent::TurnSubmitMode>>(&params, "mode")?
                    .unwrap_or_default();
            let idempotency_key: Option<String> = arg(&params, "idempotencyKey")?;
            if !core.claim_idempotent("send_modern_agent_turn", idempotency_key.as_deref()) {
                return Ok(json!({
                    "outcome": "started",
                    "turnId": "",
                    "queueLength": 0,
                    "deduplicated": true,
                }));
            }
            let launch = nova_core::agent::prepare_modern_agent_turn(
                core,
                &conversation_id,
                &prompt,
                attachments,
                mode,
                client_id.map(str::to_string),
            )
            .map_err(|err| (INVALID_PARAMS, err))?;
            let outcome_json =
                serde_json::to_value(&launch.outcome).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;

            let core = core.clone();
            tokio::spawn(async move {
                if let Err(error) = nova_core::agent::run_prepared_turn_sequence(
                    core,
                    conversation_id,
                    launch,
                )
                .await
                {
                    tracing::error!(%error, "agent turn sequence failed");
                }
            });
            Ok(outcome_json)
        }
        "cancel_modern_agent_turn" => result(nova_core::agent::cancel_modern_agent_turn(
            arg(&params, "conversationId")?,
        ).await),
        "send_branch_message" => {
            let core = core.clone();
            let parent_conversation_id: String = arg(&params, "parentConversationId")?;
            let branch_id: String = arg(&params, "branchId")?;
            let messages = arg(&params, "messages")?;
            tokio::spawn(async move {
                if let Err(error) = nova_core::agent::orchestration::branch::send_branch_message(
                    core,
                    parent_conversation_id,
                    branch_id,
                    messages,
                )
                .await
                {
                    tracing::error!(%error, "branch message failed");
                }
            });
            Ok(json!({ "started": true }))
        }
        "cancel_branch_message" => result(nova_core::agent::orchestration::branch::cancel_branch_message(
            arg(&params, "parentConversationId")?,
            arg(&params, "branchId")?,
        ).await),
        "list_agent_bundles" => result(nova_core::api::agent_config::list_agent_bundles(core)),
        "create_agent_bundle" => result(nova_core::api::agent_config::create_agent_bundle(core, arg(&params, "name")?)),
        "load_agent_bundle" => result(nova_core::api::agent_config::load_agent_bundle(core, arg(&params, "bundleId")?)),
        "save_agent_bundle" => result(nova_core::api::agent_config::save_agent_bundle(core, arg(&params, "bundle")?)),
        "delete_agent_bundle" => result(nova_core::api::agent_config::delete_agent_bundle(core, arg(&params, "bundleId")?).await),
        "get_conversation_agent" => result(nova_core::api::agent_config::get_conversation_agent(core, arg(&params, "conversationId")?).await),
        "set_conversation_agent" => result(nova_core::api::agent_config::set_conversation_agent(core, arg(&params, "conversationId")?, arg(&params, "bundleId")?).await),
        "list_agent_private_skills" => result(nova_core::api::agent_config::list_agent_private_skills(core, arg(&params, "bundleId")?)),
        "list_agent_files" => result(nova_core::api::agent_config::list_agent_files(core, arg(&params, "bundleId")?)),
        "import_agent_file" => result(nova_core::api::agent_config::import_agent_file(core, arg(&params, "bundleId")?, arg(&params, "srcPath")?)),
        "delete_agent_file" => result(nova_core::api::agent_config::delete_agent_file(core, arg(&params, "bundleId")?, arg(&params, "name")?)),
        "reveal_agent_dir" => result(nova_core::api::agent_config::reveal_agent_dir(core, arg(&params, "bundleId")?)),
        "list_scheduled_tasks" => result(nova_core::api::cron::list_scheduled_tasks(core)),
        "create_scheduled_task" => result(nova_core::api::cron::create_scheduled_task(core, arg(&params, "cron")?, arg(&params, "prompt")?, arg(&params, "recurring")?, arg(&params, "durable")?).await),
        "delete_scheduled_task" => result(nova_core::api::cron::delete_scheduled_task(core, arg(&params, "id")?).await),
        "get_workspace_diff" => result(nova_core::api::file_changes::get_workspace_diff(core, arg(&params, "conversationId")?).await),
        "get_git_repo_status" => result(nova_core::api::file_changes::get_git_repo_status(core, arg(&params, "conversationId")?).await),
        "get_workspace_git_status" => result(nova_core::api::file_changes::get_workspace_git_status(arg(&params, "workspacePath")?).await),
        "init_git_repo" => result(nova_core::api::file_changes::init_git_repo(core, arg(&params, "conversationId")?).await),
        "create_conversation" => core_result(nova_core::api::history::create_conversation(core, arg(&params, "title")?, arg(&params, "workspacePath")?, arg(&params, "idempotencyKey")?).await),
        "list_conversations" => result(nova_core::api::history::list_conversations(core).await),
        "set_conversation_pinned" => result(nova_core::api::history::set_conversation_pinned(core, arg(&params, "conversationId")?, arg(&params, "pinned")?).await),
        "export_conversation" => result(nova_core::api::history::export_conversation(core, arg(&params, "conversationId")?, arg(&params, "format")?).await),
        "export_rendered_conversation_pdf" => result(nova_core::api::history::export_rendered_conversation_pdf(core, arg(&params, "conversationId")?, arg(&params, "title")?, arg(&params, "html")?).await),
        "load_history" => result(nova_core::api::history::load_history(core, arg(&params, "conversationId")?).await),
        "replace_history" => result(nova_core::api::history::replace_history(core, arg(&params, "conversationId")?, arg(&params, "messages")?).await),
        "append_plain_chat_message" => result(nova_core::api::history::append_plain_chat_message(core, arg(&params, "conversationId")?, arg(&params, "role")?, arg(&params, "content")?).await),
        "load_conversation_tool_logs" => result(nova_core::api::history::load_conversation_tool_logs(core, arg(&params, "conversationId")?).await),
        "get_conversation_turn_traces" => result(nova_core::api::history::get_conversation_turn_traces(core, arg(&params, "conversationId")?).await),
        "clear_history" => result(nova_core::api::history::clear_history(core, arg(&params, "conversationId")?).await),
        "delete_conversation" => result(nova_core::api::history::delete_conversation(core, arg(&params, "conversationId")?).await),
        "list_memory_entries" => result(nova_core::api::history::list_memory_entries(core).await),
        "add_memory_entry" => result(nova_core::api::history::add_memory_entry(core, arg(&params, "content")?).await),
        "remove_memory_entry" => result(nova_core::api::history::remove_memory_entry(core, arg(&params, "oldText")?).await),
        "clear_memory_entries" => result(nova_core::api::history::clear_memory_entries(core).await),
        "manual_compact_conversation" => result(nova_core::api::history::manual_compact_conversation(core, arg(&params, "conversationId")?).await),
        "get_hooks_toml" => result(nova_core::api::hooks::get_hooks_toml(core)),
        "get_hooks_structured" => result(nova_core::api::hooks::get_hooks_structured(core)),
        "save_hooks_toml" => result(nova_core::api::hooks::save_hooks_toml(core, arg(&params, "content")?)),
        "add_mcp_server" => core_result(nova_core::api::mcp::add_mcp_server(core, arg(&params, "name")?, arg(&params, "config")?, arg(&params, "sourceAgent")?).await),
        "get_mcp_server" => core_result(nova_core::api::mcp::get_mcp_server(core, arg(&params, "name")?).await),
        "update_mcp_server" => core_result(nova_core::api::mcp::update_mcp_server(core, arg(&params, "oldName")?, arg(&params, "newName")?, arg(&params, "config")?).await),
        "remove_mcp_server" => core_result(nova_core::api::mcp::remove_mcp_server(core, arg(&params, "name")?).await),
        "get_mcp_server_statuses" => core_result(nova_core::api::mcp::get_mcp_server_statuses(core).await),
        "reload_all_mcp_servers" => core_result(nova_core::api::mcp::reload_all_mcp_servers(core).await),
        "set_mcp_server_enabled" => core_result(nova_core::api::mcp::set_mcp_server_enabled(core, arg(&params, "name")?, arg(&params, "enabled")?).await),
        "list_mcp_tools" => core_result(nova_core::api::mcp::list_mcp_tools(core, arg(&params, "serverName")?).await),
        "list_mcp_resources" => core_result(nova_core::api::mcp::list_mcp_resources(core, arg(&params, "serverName")?).await),
        "read_mcp_resource" => core_result(nova_core::api::mcp::read_mcp_resource(core, arg(&params, "serverName")?, arg(&params, "uri")?).await),
        "call_mcp_tool" => core_result(nova_core::api::mcp::call_mcp_tool(core, arg(&params, "serverName")?, arg(&params, "toolName")?, arg(&params, "arguments")?).await),
        "fetch_available_models" => result(nova_core::api::model_fetch::fetch_available_models(core, arg(&params, "baseUrl")?, arg(&params, "apiKey")?, arg(&params, "isFullUrl")?, arg(&params, "modelsUrlOverride")?, arg(&params, "apiFormat")?).await),
        "list_permission_rules" => core_result(nova_core::api::permission_rules::list_permission_rules(core)),
        "delete_permission_rule" => core_result(nova_core::api::permission_rules::delete_permission_rule(core, arg(&params, "signature")?)),
        "submit_permission_decision" => {
            let conversation_id: Option<String> = arg(&params, "conversationId")?;
            let request_id: String = arg(&params, "requestId")?;
            let action: String = arg(&params, "action")?;
            let Some(action_enum) =
                nova_core::agent::permissions::parse_permission_action_name(&action)
            else {
                return Err((
                    INVALID_PARAMS,
                    format!("Unknown permission action: {}", action),
                ));
            };
            core_result(
                nova_core::agent::permissions::submit_permission_decision_for_client(
                    core,
                    conversation_id.as_deref(),
                    &request_id,
                    action_enum,
                    client_id,
                )
                .map_err(nova_core::NovaCoreError::InvalidInput),
            )
        }
        "get_conversation_plan" => result(nova_core::api::plan::get_conversation_plan(core, arg(&params, "conversationId")?).await),
        "save_session_file" => result(nova_core::api::session_files::save_session_file(core, arg(&params, "conversationId")?, arg(&params, "filename")?, arg(&params, "content")?, arg(&params, "rawBytes")?).await),
        "list_session_files" => result(nova_core::api::session_files::list_session_files(core, arg(&params, "conversationId")?).await),
        "read_session_file" => result(nova_core::api::session_files::read_session_file(core, arg(&params, "conversationId")?, arg(&params, "filename")?).await),
        "get_settings" => result(nova_core::api::settings::get_settings(core)),
        "save_settings" => result(nova_core::api::settings::save_settings(core, arg(&params, "settings")?)),
        "get_model_window_tokens" => to_json(nova_core::api::settings::get_model_window_tokens(core, arg(&params, "model")?)),
        "estimate_text_tokens" => to_json(nova_core::api::settings::estimate_text_tokens(arg(&params, "text")?)),
        "get_available_terminals" => to_json(nova_core::api::settings::get_available_terminals()),
        "get_active_terminal" => to_json(nova_core::api::settings::get_active_terminal()),
        "refresh_model_catalog" => result(nova_core::api::settings::refresh_model_catalog(core).await),
        "get_model_catalog_stats" => to_json(nova_core::api::settings::get_model_catalog_stats(core)),
        "get_model_catalog" => to_json(nova_core::api::settings::get_model_catalog(arg(&params, "search")?, arg(&params, "modality")?)),
        "get_sandbox_status" => to_json(nova_core::api::settings::get_sandbox_status(core)),
        "export_diagnostic_bundle" => result(nova_core::api::settings::export_diagnostic_bundle(core, arg(&params, "targetPath")?)),
        "list_background_jobs" => result(nova_core::api::shell::list_background_jobs(arg(&params, "conversationId")?).await),
        "read_background_job_output" => result(nova_core::api::shell::read_background_job_output(arg(&params, "conversationId")?, arg(&params, "id")?, arg(&params, "tailLines")?).await),
        "kill_background_job" => result(nova_core::api::shell::kill_background_job(arg(&params, "conversationId")?, arg(&params, "id")?).await),
        "list_skills" => result(nova_core::api::skill::list_skills(core)),
        "delete_skill" => result(nova_core::api::skill::delete_skill(core, arg(&params, "path")?)),
        "read_skill_content" => result(nova_core::api::skill::read_skill_content(core, arg(&params, "path")?)),
        "list_todos" => result(nova_core::api::todo::list_todos(core, arg(&params, "conversationId")?).await),
        "get_usage_stats" => result(nova_core::api::usage::get_usage_stats(core).await),
        "list_token_usage" => result(nova_core::api::usage::list_token_usage(core, arg(&params, "limit")?).await),
        "get_conversation_usage" => result(nova_core::api::usage::get_conversation_usage(core, arg(&params, "conversationId")?).await),
        "workspace_list_directory" => result(nova_core::api::workspace::workspace_list_directory(core, arg(&params, "conversationId")?, arg(&params, "path")?)),
        "workspace_read_text_file" => result(nova_core::api::workspace::workspace_read_text_file(core, arg(&params, "conversationId")?, arg(&params, "path")?)),
        "get_workspace_root" => result(nova_core::api::workspace::get_workspace_root(core, arg(&params, "conversationId")?)),
        "get_agent_graph" => result(nova_core::api::collaboration::get_agent_graph(core, arg(&params, "conversationId")?).await),
        "get_board_channels" => result(nova_core::api::collaboration::get_board_channels(core, arg(&params, "conversationId")?).await),
        "get_board_posts" => result(nova_core::api::collaboration::get_board_posts(core, arg(&params, "conversationId")?, arg(&params, "channel").ok(), arg(&params, "threadId").ok(), arg(&params, "limit").ok()).await),
        "post_board_message" => core_result(
            nova_core::api::collaboration::post_board_message(
                core,
                nova_core::api::collaboration::PostBoardMessageInput {
                    conversation_id: arg(&params, "conversationId")?,
                    channel: arg(&params, "channel")?,
                    content: arg(&params, "content")?,
                    thread_id: arg(&params, "threadId").ok(),
                    author_nickname: arg(&params, "authorNickname").ok(),
                    author_role: arg(&params, "authorRole").ok(),
                    idempotency_key: arg(&params, "idempotencyKey")?,
                },
            )
            .await,
        ),
        "interrupt_agent" => result(nova_core::api::collaboration::interrupt_agent(core, arg(&params, "conversationId")?, arg(&params, "agentId")?).await),
        "send_agent_message" => result(nova_core::api::collaboration::send_agent_message(core, arg(&params, "conversationId")?, arg(&params, "agentId")?, arg(&params, "directive")?).await),
        _ => Err((METHOD_NOT_FOUND, format!("unknown method: {}", method))),
    }
}

/// 按 camelCase 键名提取并反序列化参数（与 Tauri 前端参数约定一致）。
fn arg<T: DeserializeOwned>(params: &Value, key: &str) -> Result<T, DispatchError> {
    let raw = params.get(key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(raw).map_err(|e| (INVALID_PARAMS, format!("参数 {} 解析失败: {}", key, e)))
}

fn result<T: Serialize>(value: Result<T, String>) -> Result<Value, DispatchError> {
    core_result(value.map_err(nova_core::NovaCoreError::Internal))
}

fn core_result<T: Serialize>(
    value: Result<T, nova_core::NovaCoreError>,
) -> Result<Value, DispatchError> {
    match value {
        Ok(v) => serde_json::to_value(v).map_err(|e| (INTERNAL_ERROR, e.to_string())),
        Err(e) => Err(e.to_json_rpc_error()),
    }
}

fn to_json<T: Serialize>(value: T) -> Result<Value, DispatchError> {
    serde_json::to_value(value).map_err(|e| (INTERNAL_ERROR, e.to_string()))
}
