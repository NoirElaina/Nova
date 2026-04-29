// 这是工具注册入口模块，定义了所有内置工具（Bash/PowerShell/File/Task/... 等）
// 以及工具发现、执行、权限检查的统一接口。
#[path = "BashTool/mod.rs"]
pub mod bash_tool;
#[path = "WriteFileTool/mod.rs"]
pub mod write_file_tool;
#[path = "GrepSearchTool/mod.rs"]
pub mod grep_search_tool;
pub mod shared;
#[path = "GlobTool/mod.rs"]
pub mod glob_tool;
#[path = "PowerShellTool/mod.rs"]
pub mod powershell_tool;
#[path = "WebFetchTool/mod.rs"]
pub mod web_fetch_tool;
#[path = "WebSearchTool/mod.rs"]
pub mod web_search_tool;
#[path = "TaskCreateTool/mod.rs"]
pub mod task_create_tool;
#[path = "TaskListTool/mod.rs"]
pub mod task_list_tool;
#[path = "TaskUpdateTool/mod.rs"]
pub mod task_update_tool;
#[path = "TaskGetTool/mod.rs"]
pub mod task_get_tool;
#[path = "TaskOutputTool/mod.rs"]
pub mod task_output_tool;
#[path = "TaskStopTool/mod.rs"]
pub mod task_stop_tool;
#[path = "TaskCreateCompatTool/mod.rs"]
pub mod task_create_compat_tool;
#[path = "TaskListCompatTool/mod.rs"]
pub mod task_list_compat_tool;
#[path = "TaskUpdateCompatTool/mod.rs"]
pub mod task_update_compat_tool;
#[path = "SkillTool/mod.rs"]
pub mod skill_tool;
#[path = "TodoWriteTool/mod.rs"]
pub mod todo_write_tool;
#[path = "ToolSearchTool/mod.rs"]
pub mod tool_search_tool;
#[path = "MCPTool/mod.rs"]
pub mod mcp_tool;
#[path = "ListMcpResourcesTool/mod.rs"]
pub mod list_mcp_resources_tool;
#[path = "ReadMcpResourceTool/mod.rs"]
pub mod read_mcp_resource_tool;
#[path = "McpAuthTool/mod.rs"]
pub mod mcp_auth_tool;
#[path = "LSPTool/mod.rs"]
pub mod lsp_tool;
#[path = "FileReadTool/mod.rs"]
pub mod file_read_tool;
#[path = "FileEditTool/mod.rs"]
pub mod file_edit_tool;
#[path = "AskUserQuestionTool/mod.rs"]
pub mod ask_user_question_tool;
#[path = "PlanForApprovalTool/mod.rs"]
pub mod plan_for_approval_tool;
#[path = "RememberGlobalMemoryTool/mod.rs"]
pub mod remember_global_memory_tool;
#[path = "ConfigTool/mod.rs"]
pub mod config_tool;
#[path = "EnterPlanModeTool/mod.rs"]
pub mod enter_plan_mode_tool;
#[path = "ExitPlanModeTool/mod.rs"]
pub mod exit_plan_mode_tool;
#[path = "RagTool/mod.rs"]
pub mod rag_tool;
#[path = "SyntheticOutputTool/mod.rs"]
pub mod synthetic_output_tool;
#[path = "SleepTool/mod.rs"]
pub mod sleep_tool;
#[path = "CronCreateTool/mod.rs"]
pub mod cron_create_tool;
#[path = "CronListTool/mod.rs"]
pub mod cron_list_tool;
#[path = "CronDeleteTool/mod.rs"]
pub mod cron_delete_tool;
#[path = "ComputerUseTool/mod.rs"]
pub mod computer_use_tool;

// Placeholder migration modules stay out of `registered_tools()` until their
// runtime bridge is complete. This avoids exposing Claude-style folders as if
// they were fully migrated Nova tools.

use crate::llm::types::Tool;
use std::collections::{BTreeMap, VecDeque};
use crate::llm::services::mcp_tools::parse_mcp_tool_name;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::task::JoinSet;

struct RegisteredTool {
    tool: fn() -> Tool,
    execute: fn(Value) -> String,
}

#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub output: String,
    pub is_error: bool,
    pub additional_messages: Vec<crate::llm::types::Message>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
}

fn find_tool_definition(name: &str) -> Option<Tool> {
    registered_tools().into_iter().find_map(|entry| {
        let tool = (entry.tool)();
        if tool.name == name {
            Some(tool)
        } else {
            None
        }
    })
}

fn validate_type(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn validate_schema_fragment(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    if let Some(expected_type) = schema.get("type").and_then(|v| v.as_str()) {
        if !validate_type(value, expected_type) {
            return Err(format!(
                "Input validation failed for '{}': expected {}, got {}",
                path,
                expected_type,
                value
            ));
        }
    }

    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array()) {
        if !enum_values.iter().any(|allowed| allowed == value) {
            return Err(format!(
                "Input validation failed for '{}': value not in enum",
                path
            ));
        }
    }

    if schema.get("type").and_then(|v| v.as_str()) == Some("object") {
        let object = value
            .as_object()
            .ok_or_else(|| format!("Input validation failed for '{}': expected object", path))?;

        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for key in required.iter().filter_map(|k| k.as_str()) {
                if !object.contains_key(key) {
                    return Err(format!(
                        "Input validation failed for '{}': missing required field '{}'",
                        path, key
                    ));
                }
            }
        }

        if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, sub_schema) in properties {
                if let Some(sub_value) = object.get(key) {
                    let sub_path = format!("{}.{}", path, key);
                    validate_schema_fragment(sub_value, sub_schema, &sub_path)?;
                }
            }
        }
    }

    if schema.get("type").and_then(|v| v.as_str()) == Some("array") {
        let array = value
            .as_array()
            .ok_or_else(|| format!("Input validation failed for '{}': expected array", path))?;

        if let Some(item_schema) = schema.get("items") {
            for (index, item) in array.iter().enumerate() {
                let sub_path = format!("{}[{}]", path, index);
                validate_schema_fragment(item, item_schema, &sub_path)?;
            }
        }
    }

    Ok(())
}

fn validate_tool_input(name: &str, input: &Value) -> Result<(), String> {
    if let Some(tool) = find_tool_definition(name) {
        return validate_schema_fragment(input, &tool.input_schema, name);
    }

    Ok(())
}

fn validate_tool_output(name: &str, output: &str) -> Result<(), String> {
    let trimmed = output.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        serde_json::from_str::<Value>(trimmed).map_err(|e| {
            format!(
                "Output validation failed for '{}': invalid JSON payload ({})",
                name, e
            )
        })?;
    }
    Ok(())
}

fn infer_is_error(output: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(output) else {
        return false;
    };

    if v
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "needs_user_input")
        .unwrap_or(false)
    {
        return false;
    }

    if v.get("ok").and_then(|ok| ok.as_bool()) == Some(false) {
        return true;
    }

    v.get("error").is_some() && v.get("ok").is_none()
}

fn is_subagent_start_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "taskcreate" | "task_create"
    )
}

fn is_subagent_stop_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "taskstop" | "task_stop"
    )
}

pub(crate) fn is_read_only_tool(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "read_file"
        | "grep_search"
        | "glob_search"
        | "web_fetch"
        | "web_search"
        | "task_list"
        | "taskget"
        | "taskoutput"
        | "task_get"
        | "task_output"
        | "tool_search"
        | "list_mcp_resources"
        | "read_mcp_resource"
        | "rag_tool"
        | "structuredoutput"
        | "structured_output"
        | "sleep"
        | "cronlist"
        | "cron_list"
        | "skill"
        | "lsp_tool" => true,
        _ => {
            if let Some((_server, tool_name)) = parse_mcp_tool_name(name) {
                let tool_lower = tool_name.to_ascii_lowercase();
                return ["read", "list", "search", "get", "fetch", "glob", "grep"]
                    .iter()
                    .any(|kw| tool_lower.contains(kw));
            }
            false
        }
    }
}

fn max_tool_use_concurrency() -> usize {
    std::env::var("NOVA_MAX_TOOL_USE_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(10)
}

fn cancelled_result_from_call(call: ToolCallRequest, reason: &str) -> ToolCallResult {
    ToolCallResult {
        id: call.id,
        name: call.name,
        input: call.input,
        output: json!({ "ok": false, "error": reason }).to_string(),
        is_error: true,
        additional_messages: Vec::new(),
        prevent_continuation: false,
        stop_reason: None,
    }
}

pub(crate) async fn execute_single_tool_call(
    app: &AppHandle,
    conversation_id: Option<&str>,
    call: ToolCallRequest,
) -> ToolCallResult {
    let ToolCallRequest { id, name, input } = call;
    let mut additional_messages = Vec::new();
    let mut prevent_continuation = false;
    let mut stop_reason: Option<String> = None;

    if is_subagent_start_tool(&name) {
        let subagent_start_hook = crate::llm::services::hooks::run_subagent_start_hooks(
            app,
            &name,
            conversation_id,
        );
        additional_messages.extend(subagent_start_hook.additional_messages);
        if subagent_start_hook.prevent_continuation {
            prevent_continuation = true;
            if stop_reason.is_none() {
                stop_reason = subagent_start_hook.stop_reason;
            }
        }
    }

    let pre_hook = crate::llm::services::hooks::run_pre_tool_use_hooks(
        app,
        &name,
        &input,
        conversation_id,
    );
    additional_messages.extend(pre_hook.additional_messages);
    if pre_hook.prevent_continuation {
        prevent_continuation = true;
        stop_reason = pre_hook.stop_reason.clone();
    }

    if let Some(err) = pre_hook.override_error {
        return ToolCallResult {
            id,
            name,
            input,
            output: json!({ "ok": false, "error": err }).to_string(),
            is_error: true,
            additional_messages,
            prevent_continuation,
            stop_reason,
        };
    }

    if let Err(e) = validate_tool_input(&name, &input) {
        let failure_hook = crate::llm::services::hooks::run_post_tool_use_failure_hooks(
            app,
            &name,
            &input,
            &e,
            conversation_id,
        );
        additional_messages.extend(failure_hook.additional_messages);
        if failure_hook.prevent_continuation {
            prevent_continuation = true;
            if stop_reason.is_none() {
                stop_reason = failure_hook.stop_reason;
            }
        }

        let output = json!({ "ok": false, "error": e }).to_string();
        return ToolCallResult {
            id,
            name,
            input,
            output,
            is_error: true,
            additional_messages,
            prevent_continuation,
            stop_reason,
        };
    }

    let output = if let Some((server_name, tool_name)) = parse_mcp_tool_name(&name) {
        execute_tool_with_app(
            app,
            conversation_id,
            "mcp_tool",
            json!({
                "server": server_name,
                "tool": tool_name,
                "arguments": input.clone(),
            }),
        )
        .await
    } else {
        execute_tool_with_app(app, conversation_id, &name, input.clone()).await
    };

    let validated_output = match validate_tool_output(&name, &output) {
        Ok(()) => output,
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    };
    let (validated_output, tool_side_channel_messages) = match name.as_str() {
        "computer_use" => computer_use_tool::postprocess_output(&validated_output),
        _ => (validated_output, Vec::new()),
    };
    additional_messages.extend(tool_side_channel_messages);

    let mut is_error = infer_is_error(&validated_output);

    if is_error {
        let failure_hook = crate::llm::services::hooks::run_post_tool_use_failure_hooks(
            app,
            &name,
            &input,
            &validated_output,
            conversation_id,
        );
        additional_messages.extend(failure_hook.additional_messages);
        if failure_hook.prevent_continuation {
            prevent_continuation = true;
            if stop_reason.is_none() {
                stop_reason = failure_hook.stop_reason;
            }
        }
    }

    let post_hook = crate::llm::services::hooks::run_post_tool_use_hooks(
        app,
        &name,
        &input,
        &validated_output,
        is_error,
        conversation_id,
    );
    additional_messages.extend(post_hook.additional_messages);
    if post_hook.prevent_continuation {
        prevent_continuation = true;
        if stop_reason.is_none() {
            stop_reason = post_hook.stop_reason;
        }
    }

    let final_output = if let Some(err) = post_hook.override_error {
        is_error = true;
        json!({ "ok": false, "error": err }).to_string()
    } else {
        validated_output
    };

    if is_subagent_stop_tool(&name) {
        let subagent_stop_hook = crate::llm::services::hooks::run_subagent_stop_hooks(
            app,
            &name,
            conversation_id,
        );
        additional_messages.extend(subagent_stop_hook.additional_messages);
        if subagent_stop_hook.prevent_continuation {
            prevent_continuation = true;
            if stop_reason.is_none() {
                stop_reason = subagent_stop_hook.stop_reason;
            }
        }
    }

    ToolCallResult {
        id,
        name,
        input,
        is_error,
        output: final_output,
        additional_messages,
        prevent_continuation,
        stop_reason,
    }
}

async fn execute_read_only_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    calls: Vec<ToolCallRequest>,
) -> Vec<ToolCallResult> {
    let total = calls.len();
    if total == 0 {
        return Vec::new();
    }

    let mut queue: VecDeque<(usize, ToolCallRequest)> =
        calls.into_iter().enumerate().collect();
    let mut in_flight: BTreeMap<usize, ToolCallRequest> = BTreeMap::new();
    let mut results_by_index: BTreeMap<usize, ToolCallResult> = BTreeMap::new();
    let mut tasks: JoinSet<(usize, ToolCallResult)> = JoinSet::new();
    let mut cascade_reason: Option<String> = None;
    let max_concurrency = max_tool_use_concurrency();
    let conversation_owned = conversation_id.map(|v| v.to_string());

    while !queue.is_empty() || !tasks.is_empty() {
        while cascade_reason.is_none() && tasks.len() < max_concurrency && !queue.is_empty() {
            let Some((index, call)) = queue.pop_front() else {
                break;
            };

            let app_clone = app.clone();
            let conversation_for_task = conversation_owned.clone();
            in_flight.insert(index, call.clone());
            tasks.spawn(async move {
                let result = execute_single_tool_call(
                    &app_clone,
                    conversation_for_task.as_deref(),
                    call,
                )
                .await;
                (index, result)
            });
        }

        let Some(joined) = tasks.join_next().await else {
            break;
        };

        if let Ok((index, result)) = joined {
            in_flight.remove(&index);
            let is_error = result.is_error;
            let error_tool_name = result.name.clone();
            results_by_index.insert(index, result);

            if cascade_reason.is_none() && is_error {
                cascade_reason = Some(format!(
                    "Cancelled: parallel tool call '{}' errored",
                    error_tool_name
                ));
                tasks.abort_all();

                while let Some(_drained) = tasks.join_next().await {
                    // Ignore aborted task outputs; unresolved calls are converted
                    // into deterministic synthetic cancellations below.
                }
                break;
            }
        }
    }

    if let Some(reason) = cascade_reason {
        for (index, call) in in_flight.into_iter() {
            results_by_index
                .entry(index)
                .or_insert_with(|| cancelled_result_from_call(call, &reason));
        }
        while let Some((index, call)) = queue.pop_front() {
            results_by_index
                .entry(index)
                .or_insert_with(|| cancelled_result_from_call(call, &reason));
        }
    } else {
        for (index, call) in in_flight.into_iter() {
            results_by_index.entry(index).or_insert_with(|| {
                cancelled_result_from_call(call, "cancelled: read-only task aborted")
            });
        }
        while let Some((index, call)) = queue.pop_front() {
            results_by_index.entry(index).or_insert_with(|| {
                cancelled_result_from_call(call, "cancelled: read-only task not executed")
            });
        }
    }

    let mut ordered_results = Vec::with_capacity(total);
    for index in 0..total {
        if let Some(result) = results_by_index.remove(&index) {
            ordered_results.push(result);
        }
    }
    ordered_results
}

async fn flush_read_only_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    batch: &mut Vec<ToolCallRequest>,
    out: &mut Vec<ToolCallResult>,
) {
    if batch.is_empty() {
        return;
    }

    let drained = std::mem::take(batch);
    let mut batch_results = execute_read_only_batch(app, conversation_id, drained).await;
    out.append(&mut batch_results);
}

pub async fn execute_tool_calls_with_app(
    app: &AppHandle,
    conversation_id: Option<&str>,
    calls: Vec<ToolCallRequest>,
) -> Vec<ToolCallResult> {
    let mut results: Vec<ToolCallResult> = Vec::with_capacity(calls.len());
    let mut read_only_batch: Vec<ToolCallRequest> = Vec::new();

    for call in calls {
        if crate::llm::cancellation::is_cancelled(conversation_id) {
            results.push(cancelled_result_from_call(call, "cancelled"));
            continue;
        }

        if is_read_only_tool(&call.name) {
            read_only_batch.push(call);
            continue;
        }

        flush_read_only_batch(app, conversation_id, &mut read_only_batch, &mut results).await;
        results.push(execute_single_tool_call(app, conversation_id, call).await);
    }

    flush_read_only_batch(app, conversation_id, &mut read_only_batch, &mut results).await;
    results
}

fn registered_tools() -> Vec<RegisteredTool> {
    vec![
        RegisteredTool {
            tool: bash_tool::tool,
            execute: bash_tool::execute,
        },
        RegisteredTool {
            tool: powershell_tool::tool,
            execute: powershell_tool::execute,
        },
        RegisteredTool {
            tool: file_read_tool::tool,
            execute: file_read_tool::execute,
        },
        RegisteredTool {
            tool: write_file_tool::tool,
            execute: write_file_tool::execute,
        },
        RegisteredTool {
            tool: file_edit_tool::tool,
            execute: file_edit_tool::execute,
        },
        RegisteredTool {
            tool: grep_search_tool::tool,
            execute: grep_search_tool::execute,
        },
        RegisteredTool {
            tool: glob_tool::tool,
            execute: glob_tool::execute,
        },
        RegisteredTool {
            tool: web_fetch_tool::tool,
            execute: web_fetch_tool::execute,
        },
        RegisteredTool {
            tool: web_search_tool::tool,
            execute: web_search_tool::execute,
        },
        RegisteredTool {
            tool: task_create_tool::tool,
            execute: task_create_tool::execute,
        },
        RegisteredTool {
            tool: task_create_compat_tool::tool,
            execute: task_create_compat_tool::execute,
        },
        RegisteredTool {
            tool: task_list_tool::tool,
            execute: task_list_tool::execute,
        },
        RegisteredTool {
            tool: task_list_compat_tool::tool,
            execute: task_list_compat_tool::execute,
        },
        RegisteredTool {
            tool: task_update_tool::tool,
            execute: task_update_tool::execute,
        },
        RegisteredTool {
            tool: task_update_compat_tool::tool,
            execute: task_update_compat_tool::execute,
        },
        RegisteredTool {
            tool: task_get_tool::tool,
            execute: task_get_tool::execute,
        },
        RegisteredTool {
            tool: task_output_tool::tool,
            execute: task_output_tool::execute,
        },
        RegisteredTool {
            tool: task_stop_tool::tool,
            execute: task_stop_tool::execute,
        },
        RegisteredTool {
            tool: skill_tool::tool,
            execute: skill_tool::execute,
        },
        RegisteredTool {
            tool: todo_write_tool::tool,
            execute: todo_write_tool::execute,
        },
        RegisteredTool {
            tool: tool_search_tool::tool,
            execute: tool_search_tool::execute,
        },
        RegisteredTool {
            tool: mcp_tool::tool,
            execute: mcp_tool::execute,
        },
        RegisteredTool {
            tool: list_mcp_resources_tool::tool,
            execute: list_mcp_resources_tool::execute,
        },
        RegisteredTool {
            tool: read_mcp_resource_tool::tool,
            execute: read_mcp_resource_tool::execute,
        },
        RegisteredTool {
            tool: mcp_auth_tool::tool,
            execute: mcp_auth_tool::execute,
        },
        RegisteredTool {
            tool: lsp_tool::tool,
            execute: lsp_tool::execute,
        },
        RegisteredTool {
            tool: ask_user_question_tool::tool,
            execute: ask_user_question_tool::execute,
        },
        RegisteredTool {
            tool: plan_for_approval_tool::tool,
            execute: plan_for_approval_tool::execute,
        },
        RegisteredTool {
            tool: remember_global_memory_tool::tool,
            execute: remember_global_memory_tool::execute,
        },
        RegisteredTool {
            tool: enter_plan_mode_tool::tool,
            execute: enter_plan_mode_tool::execute,
        },
        RegisteredTool {
            tool: exit_plan_mode_tool::tool,
            execute: exit_plan_mode_tool::execute,
        },
        RegisteredTool {
            tool: config_tool::tool,
            execute: config_tool::execute,
        },
        RegisteredTool {
            tool: rag_tool::tool,
            execute: rag_tool::execute,
        },
        RegisteredTool {
            tool: synthetic_output_tool::tool,
            execute: synthetic_output_tool::execute,
        },
        RegisteredTool {
            tool: sleep_tool::tool,
            execute: sleep_tool::execute,
        },
        RegisteredTool {
            tool: cron_create_tool::tool,
            execute: cron_create_tool::execute,
        },
        RegisteredTool {
            tool: cron_list_tool::tool,
            execute: cron_list_tool::execute,
        },
        RegisteredTool {
            tool: cron_delete_tool::tool,
            execute: cron_delete_tool::execute,
        },
        RegisteredTool {
            tool: computer_use_tool::tool,
            execute: computer_use_tool::execute,
        },
    ]
}

// 取当前注册工具列表，用于在 LLM 提示里传给模型，告诉模型可调用哪些功能。
pub fn get_available_tools() -> Vec<Tool> {
    registered_tools()
        .into_iter()
        .map(|entry| (entry.tool)())
        .collect()
}

// 在后端直接执行工具，输入来自模型返回的 tool call 名称和参数，只在同步模式下使用。
pub fn execute_tool(name: &str, input: Value) -> String {
    for entry in registered_tools() {
        let tool = (entry.tool)();
        if tool.name == name {
            return (entry.execute)(input);
        }
    }

    format!("Unknown tool: {}", name)
}

// 在带 AppHandle 的环境中执行工具，附带权限校验和 MCP 代理能力。
// 若权限拒绝返回特殊 JSON payload；允许则执行工具。
pub async fn execute_tool_with_app(
    app: &AppHandle,
    conversation_id: Option<&str>,
    name: &str,
    input: Value,
) -> String {
    match crate::llm::utils::permissions::enforce_tool_permission(
        app,
        conversation_id,
        name,
        &input,
    ) {
        crate::llm::utils::permissions::PermissionEnforcement::Allow => {}
        crate::llm::utils::permissions::PermissionEnforcement::Deny(e) => {
            return serde_json::json!({ "ok": false, "error": e }).to_string();
        }
        crate::llm::utils::permissions::PermissionEnforcement::AskUser {
            request_id,
            payload,
        } => {
            if let Err(e) = shared::permission_runtime::await_permission_and_recheck(
                app,
                conversation_id,
                name,
                &input,
                request_id,
                payload,
            )
            .await
            {
                return serde_json::json!({ "ok": false, "error": e }).to_string();
            }
        }
    }

    match name {
        "Skill" | "skill" => skill_tool::execute_with_app(app, input).await,
        "config_tool" => config_tool::execute_with_app(app, input).await,
        "rag_tool" => rag_tool::execute_with_app(app, input).await,
        "CronCreate" | "cron_create" => cron_create_tool::execute_with_app(app, input).await,
        "CronList" | "cron_list" => cron_list_tool::execute_with_app(app, input).await,
        "CronDelete" | "cron_delete" => cron_delete_tool::execute_with_app(app, input).await,
        "mcp_tool" => mcp_tool::execute_with_app(app, input).await,
        "list_mcp_resources" => list_mcp_resources_tool::execute_with_app(app, input).await,
        "read_mcp_resource" => read_mcp_resource_tool::execute_with_app(app, input).await,
        "mcp_auth" => mcp_auth_tool::execute_with_app(app, conversation_id, input).await,
        "lsp_tool" => lsp_tool::execute_with_app(app, conversation_id, input).await,
        "remember_global_memory" => remember_global_memory_tool::execute_with_app(app, input).await,
        "computer_use" => computer_use_tool::execute_with_app(app, conversation_id, input).await,
        _ => execute_tool(name, input),
    }
}
