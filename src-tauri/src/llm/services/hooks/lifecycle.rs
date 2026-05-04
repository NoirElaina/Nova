use tauri::AppHandle;

use super::config::HookConfig;
use super::shared::context_message;
use super::types::HookOutcome;

const SESSION_START_HOOK_CONTEXT_KEY: &str = "NOVA_SESSION_START_HOOK_CONTEXT";
const USER_PROMPT_SUBMIT_HOOK_CONTEXT_KEY: &str = "NOVA_USER_PROMPT_SUBMIT_HOOK_CONTEXT";
const PRE_COMPACT_HOOK_CONTEXT_KEY: &str = "NOVA_PRE_COMPACT_HOOK_CONTEXT";
const POST_COMPACT_HOOK_CONTEXT_KEY: &str = "NOVA_POST_COMPACT_HOOK_CONTEXT";
const SUBAGENT_START_HOOK_CONTEXT_KEY: &str = "NOVA_SUBAGENT_START_HOOK_CONTEXT";
const SUBAGENT_STOP_HOOK_CONTEXT_KEY: &str = "NOVA_SUBAGENT_STOP_HOOK_CONTEXT";
const SESSION_END_HOOK_CONTEXT_KEY: &str = "NOVA_SESSION_END_HOOK_CONTEXT";
const ERROR_HOOK_CONTEXT_KEY: &str = "NOVA_ERROR_HOOK_CONTEXT";


// 如果配置里存在某个 key，就把对应内容包装成一条上下文消息，追加到结果里
fn append_context_hook_message(out: &mut HookOutcome, config: &HookConfig, key: &str, prefix: &str) {
    if let Some(extra) = config.value(key) {
        out.additional_messages.push(context_message(prefix, extra));
    }
}

pub fn run_session_start_hooks(app: &AppHandle, _conversation_id: Option<&str>) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    append_context_hook_message(
        &mut out,
        &config,
        SESSION_START_HOOK_CONTEXT_KEY,
        "[SessionStart]",
    );
    out
}

pub fn run_user_prompt_submit_hooks(app: &AppHandle, _conversation_id: Option<&str>) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    append_context_hook_message(
        &mut out,
        &config,
        USER_PROMPT_SUBMIT_HOOK_CONTEXT_KEY,
        "[UserPromptSubmit]",
    );
    out
}

pub fn run_pre_compact_hooks(app: &AppHandle, _conversation_id: Option<&str>) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    append_context_hook_message(
        &mut out,
        &config,
        PRE_COMPACT_HOOK_CONTEXT_KEY,
        "[PreCompact]",
    );
    out
}

pub fn run_post_compact_hooks(app: &AppHandle, _conversation_id: Option<&str>) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    append_context_hook_message(
        &mut out,
        &config,
        POST_COMPACT_HOOK_CONTEXT_KEY,
        "[PostCompact]",
    );
    out
}

pub fn run_subagent_start_hooks(
    app: &AppHandle,
    subagent_name: &str,
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    if let Some(extra) = config.value(SUBAGENT_START_HOOK_CONTEXT_KEY) {
        out.additional_messages.push(context_message(
            "[SubagentStart]",
            &format!("{} (name: {})", extra, subagent_name),
        ));
    }
    out
}

pub fn run_subagent_stop_hooks(
    app: &AppHandle,
    subagent_name: &str,
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    if let Some(extra) = config.value(SUBAGENT_STOP_HOOK_CONTEXT_KEY) {
        out.additional_messages.push(context_message(
            "[SubagentStop]",
            &format!("{} (name: {})", extra, subagent_name),
        ));
    }
    out
}

pub fn run_session_end_hooks(
    app: &AppHandle,
    stop_reason: &str,
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    if let Some(extra) = config.value(SESSION_END_HOOK_CONTEXT_KEY) {
        out.stop_reason = Some(format!("{} | [SessionEnd] {}", stop_reason, extra));
    }
    out
}

pub fn run_error_hooks(app: &AppHandle, error: &str, _conversation_id: Option<&str>) -> HookOutcome {
    let config = HookConfig::from_app(app);
    let mut out = HookOutcome::default();
    if let Some(extra) = config.value(ERROR_HOOK_CONTEXT_KEY) {
        out.override_error = Some(format!("{} | [ErrorHook] {}", error, extra));
    }
    out
}
