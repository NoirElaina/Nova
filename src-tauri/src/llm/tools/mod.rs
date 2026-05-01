// 这是工具注册入口模块，定义了所有内置工具（Bash/PowerShell/File/Task/... 等）
// 以及工具发现、执行、权限检查的统一接口。
macro_rules! declare_builtin_tools {
    ($( $module:ident => $path:literal ),* $(,)?) => {
        $(
            #[path = $path]
            pub mod $module;
        )*

        fn builtin_tool_registrations() -> Vec<ToolRegistration> {
            vec![
                $(
                    $module::registration(),
                )*
            ]
        }
    };
}

declare_builtin_tools! {
    bash_tool => "BashTool/mod.rs",
    write_file_tool => "WriteFileTool/mod.rs",
    grep_search_tool => "GrepSearchTool/mod.rs",
    glob_tool => "GlobTool/mod.rs",
    powershell_tool => "PowerShellTool/mod.rs",
    web_fetch_tool => "WebFetchTool/mod.rs",
    web_search_tool => "WebSearchTool/mod.rs",
    task_create_tool => "TaskCreateTool/mod.rs",
    task_list_tool => "TaskListTool/mod.rs",
    task_update_tool => "TaskUpdateTool/mod.rs",
    task_get_tool => "TaskGetTool/mod.rs",
    task_output_tool => "TaskOutputTool/mod.rs",
    task_stop_tool => "TaskStopTool/mod.rs",
    skill_tool => "SkillTool/mod.rs",
    todo_write_tool => "TodoWriteTool/mod.rs",
    tool_search_tool => "ToolSearchTool/mod.rs",
    list_mcp_resources_tool => "ListMcpResourcesTool/mod.rs",
    read_mcp_resource_tool => "ReadMcpResourceTool/mod.rs",
    mcp_auth_tool => "McpAuthTool/mod.rs",
    lsp_tool => "LSPTool/mod.rs",
    file_read_tool => "FileReadTool/mod.rs",
    file_edit_tool => "FileEditTool/mod.rs",
    ask_user_question_tool => "AskUserQuestionTool/mod.rs",
    plan_for_approval_tool => "PlanForApprovalTool/mod.rs",
    remember_global_memory_tool => "RememberGlobalMemoryTool/mod.rs",
    config_tool => "ConfigTool/mod.rs",
    enter_plan_mode_tool => "EnterPlanModeTool/mod.rs",
    exit_plan_mode_tool => "ExitPlanModeTool/mod.rs",
    rag_tool => "RagTool/mod.rs",
    synthetic_output_tool => "SyntheticOutputTool/mod.rs",
    sleep_tool => "SleepTool/mod.rs",
    cron_create_tool => "CronCreateTool/mod.rs",
    cron_list_tool => "CronListTool/mod.rs",
    cron_delete_tool => "CronDeleteTool/mod.rs",
    computer_use_tool => "ComputerUseTool/mod.rs",
}

pub mod shared;
pub mod process;

use crate::llm::types::{Message, Tool};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::task::JoinSet;

pub(crate) type AppExecuteFuture = Pin<Box<dyn Future<Output = String> + Send>>;
pub(crate) type AppExecuteFn = fn(AppHandle, Option<String>, Value) -> AppExecuteFuture;
pub(crate) type PostprocessFn = fn(&str) -> (String, Vec<Message>);
pub(crate) type PermissionFn = fn(&Value) -> Option<ToolPermissionDescriptor>;

#[derive(Debug, Clone)]
pub(crate) struct ToolPermissionDescriptor {
    // signature: 当前敏感操作的稳定签名，用于会话内权限复用与去重。
    pub signature: String,
    // preview: 展示给用户看的简短操作摘要。
    pub preview: String,
    // warning: 风险提示文案；为空表示仅记录该操作，不额外提示风险。
    pub warning: Option<String>,
    // needs_approval: 是否必须先经过用户审批。
    pub needs_approval: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ToolRegistration {
    // tool: 暴露给模型看的静态定义（name/description/schema）。
    tool: fn() -> Tool,
    // execute: 不依赖 AppHandle 的同步执行入口。
    execute: fn(Value) -> String,
    // execute_with_app: 需要 AppHandle / async / 会话上下文时使用。
    execute_with_app: Option<AppExecuteFn>,
    // postprocess: 执行完成后补充 side-channel 消息或清洗输出。
    postprocess: Option<PostprocessFn>,
    // permission: 工具自己的权限描述函数；内置工具不再走按名字兜底。
    permission: Option<PermissionFn>,
    // read_only: 只读工具可进入批量并发执行队列。
    read_only: bool,
}

pub(crate) const fn sync_tool(
    tool: fn() -> Tool,
    execute: fn(Value) -> String,
    read_only: bool,
    permission: Option<PermissionFn>,
) -> ToolRegistration {
    // sync_tool: 适合同步工具；权限策略也必须在这里显式声明。
    ToolRegistration {
        tool,
        execute,
        execute_with_app: None,
        postprocess: None,
        permission,
        read_only,
    }
}

pub(crate) const fn app_tool(
    tool: fn() -> Tool,
    execute: fn(Value) -> String,
    execute_with_app: AppExecuteFn,
    read_only: bool,
    permission: Option<PermissionFn>,
) -> ToolRegistration {
    // app_tool: 适合异步或依赖 AppHandle 的工具；权限策略同样显式声明。
    ToolRegistration {
        tool,
        execute,
        execute_with_app: Some(execute_with_app),
        postprocess: None,
        permission,
        read_only,
    }
}

pub(crate) const fn app_tool_with_extras(
    tool: fn() -> Tool,
    execute: fn(Value) -> String,
    execute_with_app: AppExecuteFn,
    read_only: bool,
    permission: Option<PermissionFn>,
    postprocess: Option<PostprocessFn>,
) -> ToolRegistration {
    // app_tool_with_extras: 在 app_tool 基础上再挂权限和后处理扩展点。
    ToolRegistration {
        tool,
        execute,
        execute_with_app: Some(execute_with_app),
        postprocess,
        permission,
        read_only,
    }
}
fn registered_tools() -> &'static [ToolRegistration] {
    static REGISTERED_TOOLS: OnceLock<Vec<ToolRegistration>> = OnceLock::new();
    // REGISTERED_TOOLS: 进程级缓存，避免每次请求都重新构建注册表。
    REGISTERED_TOOLS
        .get_or_init(builtin_tool_registrations)
        .as_slice()
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
    registered_tools().iter().find_map(|entry| {
        let tool = (entry.tool)();
        if tool.name == name {
            Some(tool)
        } else {
            None
        }
    })
}

fn find_registered_tool(name: &str) -> Option<ToolRegistration> {
    registered_tools().iter().copied().find(|entry| {
        let tool = (entry.tool)();
        tool.name == name
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
    name == task_create_tool::tool().name
}

fn is_subagent_stop_tool(name: &str) -> bool {
    name == task_stop_tool::tool().name
}

pub(crate) fn is_read_only_tool(name: &str) -> bool {
    if let Some(entry) = find_registered_tool(name) {
        return entry.read_only;
    }

    if let Some(read_only) = crate::llm::services::mcp_tools::dynamic_tool_read_only(name) {
        return read_only;
    }

    false
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

    let output = execute_tool_with_app(app, conversation_id, &name, input.clone()).await;

    let validated_output = match validate_tool_output(&name, &output) {
        Ok(()) => output,
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    };
    let (validated_output, tool_side_channel_messages) = match find_registered_tool(&name)
        .and_then(|entry| entry.postprocess)
    {
        Some(postprocess) => postprocess(&validated_output),
        None => (validated_output, Vec::new()),
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

    if is_error {
        let error_text = serde_json::from_str::<serde_json::Value>(&final_output)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| final_output.clone());
        crate::llm::utils::error_event::emit_backend_error(
            app,
            "tool.execute",
            format!("工具 {} 执行失败：{}", name, error_text),
            Some(name.as_str()),
        );
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

pub(crate) fn permission_descriptor_for_tool(
    name: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    find_registered_tool(name)
        .and_then(|entry| entry.permission.and_then(|permission| permission(input)))
}

// 取当前注册工具列表，用于在 LLM 提示里传给模型，告诉模型可调用哪些功能。
pub fn get_available_tools() -> Vec<Tool> {
    registered_tools()
        .iter()
        .map(|entry| (entry.tool)())
        .collect()
}

// 在后端直接执行工具，输入来自模型返回的 tool call 名称和参数，只在同步模式下使用。
pub fn execute_tool(name: &str, input: Value) -> String {
    if let Some(entry) = find_registered_tool(name) {
        return (entry.execute)(input);
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

    if let Some(output) =
        crate::llm::services::mcp_tools::execute_dynamic_with_app(app, name, input.clone()).await
    {
        return output;
    }

    if let Some(entry) = find_registered_tool(name) {
        if let Some(execute_with_app) = entry.execute_with_app {
            return execute_with_app(app.clone(), conversation_id.map(str::to_string), input).await;
        }
        return (entry.execute)(input);
    }

    format!("Unknown tool: {}", name)
}
