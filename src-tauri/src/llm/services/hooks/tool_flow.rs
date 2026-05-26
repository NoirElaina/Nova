use tauri::AppHandle;

use super::config::HookConfig;
use super::shared::context_message;
use super::types::HookOutcome;

pub fn run_pre_tool_use_hooks(
    app: &AppHandle,
    tool_name: &str,
    _input: &serde_json::Value,
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = match HookConfig::from_app(app) {
        Ok(config) => config,
        Err(error) => return HookOutcome::from_error(error),
    };
    let mut out = HookOutcome::default();
    let deny_list = config.csv_lower_list("NOVA_PRE_TOOL_DENY_TOOLS");
    if !deny_list.is_empty() {
        let tool_lower = tool_name.to_ascii_lowercase();
        if deny_list.iter().any(|name| name == &tool_lower) {
            out.override_error = Some(format!(
                "Blocked by PreToolUse hook: tool '{}' is deny-listed via NOVA_PRE_TOOL_DENY_TOOLS",
                tool_name
            ));
        }
    }

    if let Some(extra) = config.value("NOVA_PRE_TOOL_CONTEXT") {
        out.additional_messages
            .push(context_message("[PreToolUse]", extra));
    }

    out
}

pub fn run_post_tool_use_hooks(
    app: &AppHandle,
    _tool_name: &str,
    _input: &serde_json::Value,
    output: &str,
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = match HookConfig::from_app(app) {
        Ok(config) => config,
        Err(error) => return HookOutcome::from_error(error),
    };
    let mut out = HookOutcome::default();

    if let Some(extra) = config.value("NOVA_POST_TOOL_CONTEXT") {
        out.additional_messages
            .push(context_message("[PostToolUse]", extra));
    }

    if let Some(pattern) = config.value("NOVA_POST_TOOL_BLOCK_PATTERN") {
        if output.contains(pattern) {
            out.prevent_continuation = true;
            out.stop_reason = Some(format!(
                "PostToolUse hook stopped continuation because tool output matched pattern '{}'",
                pattern
            ));
        }
    }

    out
}

pub fn run_post_tool_use_failure_hooks(
    app: &AppHandle,
    tool_name: &str,
    _input: &serde_json::Value,
    error: &str,
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = match HookConfig::from_app(app) {
        Ok(config) => config,
        Err(error) => return HookOutcome::from_error(error),
    };
    let mut out = HookOutcome::default();

    if let Some(extra) = config.value("NOVA_POST_TOOL_FAILURE_CONTEXT") {
        out.additional_messages
            .push(context_message("[PostToolUseFailure]", extra));
    }

    if config.truthy("NOVA_POST_TOOL_FAILURE_STOP") || config.truthy("NOVA_POST_TOOL_STOP_ON_ERROR")
    {
        out.prevent_continuation = true;
        out.stop_reason = Some(format!(
            "PostToolUseFailure hook stopped continuation after '{}' failed: {}",
            tool_name, error
        ));
    }

    out
}
