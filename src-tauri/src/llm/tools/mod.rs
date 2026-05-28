#[path = "LspTools/mod.rs"]
pub mod language_server_tools;
#[path = "PatchTool/mod.rs"]
pub mod patch_tools;

// 这是工具注册入口模块，定义了所有内置工具（Bash/PowerShell/File/Task/... 等）
// 以及工具发现、执行、权限检查的统一接口。
macro_rules! declare_builtin_tools {
    ($( $module:ident => $path:literal ),* $(,)?) => {
        $(
            #[path = $path]
            pub mod $module;
        )*

        fn builtin_tool_registrations() -> Vec<ToolRegistration> {
            let mut tools = vec![
                $(
                    $module::registration(),
                )*
            ];
            tools.extend(language_server_tools::registrations());
            tools.extend(patch_tools::registrations());
            tools
        }
    };
}

declare_builtin_tools! {
    bash_tool => "BashTool/mod.rs",
    reset_shell_session_tool => "ResetShellSessionTool/mod.rs",
    write_file_tool => "WriteFileTool/mod.rs",
    grep_search_tool => "GrepSearchTool/mod.rs",
    glob_tool => "GlobTool/mod.rs",
    powershell_tool => "PowerShellTool/mod.rs",
    web_fetch_tool => "WebFetchTool/mod.rs",
    web_search_tool => "WebSearchTool/mod.rs",
    nova_browser_navigate_tool => "NovaBrowserTool/navigate.rs",
    nova_browser_snapshot_tool => "NovaBrowserTool/snapshot.rs",
    nova_browser_click_tool => "NovaBrowserTool/click.rs",
    nova_browser_type_tool => "NovaBrowserTool/type_text.rs",
    nova_browser_reset_tool => "NovaBrowserTool/reset.rs",
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
    file_read_tool => "FileReadTool/mod.rs",
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

use crate::llm::types::{Message, Tool};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::task::JoinSet;

pub(crate) type ToolExecResult = Result<ToolOutcome, ToolFailure>;
pub(crate) type AppExecuteFuture = Pin<Box<dyn Future<Output = ToolExecResult> + Send>>;
pub(crate) type AppExecuteFn = fn(AppHandle, Option<String>, Value) -> AppExecuteFuture;
pub(crate) type PermissionFn = fn(&Value) -> Option<ToolPermissionDescriptor>;

#[derive(Debug, Clone)]
pub(crate) struct ToolOutcome {
    pub output: String,
    pub additional_messages: Vec<Message>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
}

impl ToolOutcome {
    pub(crate) fn text(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            additional_messages: Vec::new(),
            prevent_continuation: false,
            stop_reason: None,
        }
    }

    pub(crate) fn json(value: Value) -> Self {
        Self::text(value.to_string())
    }

    pub(crate) fn with_additional_messages(mut self, messages: Vec<Message>) -> Self {
        self.additional_messages = messages;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolFailureKind {
    Execution,
    InvalidInput,
    PermissionDenied,
    UnknownTool,
    Cancelled,
    Mcp,
    Hook,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolFailure {
    pub message: String,
    pub kind: ToolFailureKind,
    pub additional_messages: Vec<Message>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
    pub suppress_backend_error: bool,
}

impl ToolFailure {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolFailureKind::Execution,
            additional_messages: Vec::new(),
            prevent_continuation: false,
            stop_reason: None,
            suppress_backend_error: false,
        }
    }

    pub(crate) fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(message).with_kind(ToolFailureKind::InvalidInput)
    }

    pub(crate) fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(message).with_kind(ToolFailureKind::PermissionDenied)
    }

    pub(crate) fn unknown_tool(message: impl Into<String>) -> Self {
        Self::new(message).with_kind(ToolFailureKind::UnknownTool)
    }

    pub(crate) fn cancelled(message: impl Into<String>) -> Self {
        Self::new(message)
            .with_kind(ToolFailureKind::Cancelled)
            .suppress_backend_error()
    }

    pub(crate) fn mcp(message: impl Into<String>) -> Self {
        Self::new(message).with_kind(ToolFailureKind::Mcp)
    }

    pub(crate) fn hook(message: impl Into<String>) -> Self {
        Self::new(message).with_kind(ToolFailureKind::Hook)
    }

    pub(crate) fn with_kind(mut self, kind: ToolFailureKind) -> Self {
        self.kind = kind;
        self
    }

    pub(crate) fn suppress_backend_error(mut self) -> Self {
        self.suppress_backend_error = true;
        self
    }
}

impl From<String> for ToolFailure {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for ToolFailure {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

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
    // execute_with_app: 唯一执行入口，始终携带 AppHandle / conversation_id / workspace context。
    execute_with_app: AppExecuteFn,
    // permission: 工具自己的权限描述函数；内置工具不再走按名字兜底。
    permission: Option<PermissionFn>,
    // read_only: 只读工具可进入批量并发执行队列。
    read_only: bool,
}

pub(crate) const fn app_tool(
    tool: fn() -> Tool,
    execute_with_app: AppExecuteFn,
    read_only: bool,
    permission: Option<PermissionFn>,
) -> ToolRegistration {
    ToolRegistration {
        tool,
        execute_with_app,
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

fn validate_tool_input(name: &str, input: &Value) -> Result<(), String> {
    if let Some(tool) = find_tool_definition(name) {
        let validator = jsonschema::validator_for(&tool.input_schema)
            .map_err(|error| format!("Tool schema validation failed for '{}': {}", name, error))?;
        let errors = validator.iter_errors(input).take(4).collect::<Vec<_>>();
        if errors.is_empty() {
            return Ok(());
        }

        let detail = errors
            .iter()
            .map(|error| {
                let instance_path = error.instance_path().to_string();
                let path = if instance_path.is_empty() {
                    name.to_string()
                } else {
                    format!("{}{}", name, instance_path)
                };
                format!("{}: {}", path, error)
            })
            .collect::<Vec<_>>()
            .join("; ");

        return Err(format!(
            "Input validation failed for '{}': {}",
            name, detail
        ));
    }

    Ok(())
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
        output: reason.to_string(),
        is_error: true,
        additional_messages: Vec::new(),
        prevent_continuation: false,
        stop_reason: None,
    }
}

fn merge_controls(
    additional_messages: &mut Vec<Message>,
    prevent_continuation: &mut bool,
    stop_reason: &mut Option<String>,
    messages: Vec<Message>,
    should_prevent: bool,
    reason: Option<String>,
) {
    additional_messages.extend(messages);
    if should_prevent {
        *prevent_continuation = true;
        if stop_reason.is_none() {
            *stop_reason = reason;
        }
    }
}

fn emit_tool_failure(app: &AppHandle, name: &str, failure: &ToolFailure) {
    if failure.suppress_backend_error || failure.kind == ToolFailureKind::Cancelled {
        return;
    }

    crate::llm::utils::error_event::emit_backend_error(
        app,
        "tool.execute",
        format!("工具 {} 执行失败：{}", name, failure.message),
        Some(name),
    );
}

fn finalize_failure_result(
    app: &AppHandle,
    conversation_id: Option<&str>,
    id: String,
    name: String,
    input: Value,
    mut failure: ToolFailure,
    mut additional_messages: Vec<Message>,
    mut prevent_continuation: bool,
    mut stop_reason: Option<String>,
) -> ToolCallResult {
    merge_controls(
        &mut additional_messages,
        &mut prevent_continuation,
        &mut stop_reason,
        failure.additional_messages.clone(),
        failure.prevent_continuation,
        failure.stop_reason.clone(),
    );

    let failure_hook = crate::llm::services::hooks::run_post_tool_use_failure_hooks(
        app,
        &name,
        &input,
        &failure.message,
        conversation_id,
    );
    merge_controls(
        &mut additional_messages,
        &mut prevent_continuation,
        &mut stop_reason,
        failure_hook.additional_messages,
        failure_hook.prevent_continuation,
        failure_hook.stop_reason,
    );
    if let Some(err) = failure_hook.override_error {
        failure = ToolFailure::hook(err);
    }

    emit_tool_failure(app, &name, &failure);

    ToolCallResult {
        id,
        name,
        input,
        output: failure.message,
        is_error: true,
        additional_messages,
        prevent_continuation,
        stop_reason,
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
        let subagent_start_hook =
            crate::llm::services::hooks::run_subagent_start_hooks(app, &name, conversation_id);
        additional_messages.extend(subagent_start_hook.additional_messages);
        if subagent_start_hook.prevent_continuation {
            prevent_continuation = true;
            if stop_reason.is_none() {
                stop_reason = subagent_start_hook.stop_reason;
            }
        }
    }

    let pre_hook =
        crate::llm::services::hooks::run_pre_tool_use_hooks(app, &name, &input, conversation_id);
    additional_messages.extend(pre_hook.additional_messages);
    if pre_hook.prevent_continuation {
        prevent_continuation = true;
        stop_reason = pre_hook.stop_reason.clone();
    }

    if let Some(err) = pre_hook.override_error {
        return finalize_failure_result(
            app,
            conversation_id,
            id,
            name,
            input,
            ToolFailure::hook(err),
            additional_messages,
            prevent_continuation,
            stop_reason,
        );
    }

    if let Err(e) = validate_tool_input(&name, &input) {
        return finalize_failure_result(
            app,
            conversation_id,
            id,
            name,
            input,
            ToolFailure::invalid_input(e),
            additional_messages,
            prevent_continuation,
            stop_reason,
        );
    }

    let outcome = match execute_tool_with_app(app, conversation_id, &name, input.clone()).await {
        Ok(outcome) => outcome,
        Err(failure) => {
            return finalize_failure_result(
                app,
                conversation_id,
                id,
                name,
                input,
                failure,
                additional_messages,
                prevent_continuation,
                stop_reason,
            );
        }
    };

    let tool_output = outcome.output;
    merge_controls(
        &mut additional_messages,
        &mut prevent_continuation,
        &mut stop_reason,
        outcome.additional_messages,
        outcome.prevent_continuation,
        outcome.stop_reason,
    );

    let post_hook = crate::llm::services::hooks::run_post_tool_use_hooks(
        app,
        &name,
        &input,
        &tool_output,
        conversation_id,
    );
    merge_controls(
        &mut additional_messages,
        &mut prevent_continuation,
        &mut stop_reason,
        post_hook.additional_messages,
        post_hook.prevent_continuation,
        post_hook.stop_reason,
    );
    if let Some(err) = post_hook.override_error {
        return finalize_failure_result(
            app,
            conversation_id,
            id,
            name,
            input,
            ToolFailure::hook(err),
            additional_messages,
            prevent_continuation,
            stop_reason,
        );
    }

    if is_subagent_stop_tool(&name) {
        let subagent_stop_hook =
            crate::llm::services::hooks::run_subagent_stop_hooks(app, &name, conversation_id);
        merge_controls(
            &mut additional_messages,
            &mut prevent_continuation,
            &mut stop_reason,
            subagent_stop_hook.additional_messages,
            subagent_stop_hook.prevent_continuation,
            subagent_stop_hook.stop_reason,
        );
    }

    ToolCallResult {
        id,
        name,
        input,
        is_error: false,
        output: tool_output,
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

    let mut queue: VecDeque<(usize, ToolCallRequest)> = calls.into_iter().enumerate().collect();
    let mut in_flight: BTreeMap<usize, ToolCallRequest> = BTreeMap::new();
    let mut results_by_index: BTreeMap<usize, ToolCallResult> = BTreeMap::new();
    let mut tasks: JoinSet<(usize, ToolCallResult)> = JoinSet::new();
    let mut cancellation_reason: Option<String> = None;
    let max_concurrency = max_tool_use_concurrency();
    let conversation_owned = conversation_id.map(|v| v.to_string());

    while !queue.is_empty() || !tasks.is_empty() {
        // 用户取消时立即中止所有已起飞的并发任务。
        if crate::llm::cancellation::is_cancelled(conversation_id) {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            cancellation_reason = Some("cancelled".into());
            break;
        }

        while tasks.len() < max_concurrency && !queue.is_empty() {
            let Some((index, call)) = queue.pop_front() else {
                break;
            };

            let app_clone = app.clone();
            let conversation_for_task = conversation_owned.clone();
            in_flight.insert(index, call.clone());
            tasks.spawn(async move {
                let result =
                    execute_single_tool_call(&app_clone, conversation_for_task.as_deref(), call)
                        .await;
                (index, result)
            });
        }

        let Some(joined) = tasks.join_next().await else {
            break;
        };

        if let Ok((index, result)) = joined {
            in_flight.remove(&index);
            results_by_index.insert(index, result);
        }
    }

    if let Some(reason) = cancellation_reason {
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

// 在带 AppHandle 的环境中执行工具，附带权限校验和 MCP 代理能力。
pub(crate) async fn execute_tool_with_app(
    app: &AppHandle,
    conversation_id: Option<&str>,
    name: &str,
    input: Value,
) -> ToolExecResult {
    match crate::llm::utils::permissions::enforce_tool_permission(
        app,
        conversation_id,
        name,
        &input,
    ) {
        crate::llm::utils::permissions::PermissionEnforcement::Allow => {}
        crate::llm::utils::permissions::PermissionEnforcement::Deny(e) => {
            return Err(ToolFailure::permission_denied(e));
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
                return Err(ToolFailure::permission_denied(e));
            }
        }
    }

    if let Some(output) =
        crate::llm::services::mcp_tools::execute_dynamic_with_app(app, name, input.clone()).await
    {
        return output;
    }

    if let Some(entry) = find_registered_tool(name) {
        return (entry.execute_with_app)(app.clone(), conversation_id.map(str::to_string), input)
            .await;
    }

    Err(ToolFailure::unknown_tool(format!("Unknown tool: {}", name)))
}
