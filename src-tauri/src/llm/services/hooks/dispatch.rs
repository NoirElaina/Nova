//! 挂钩事件分发：12 个生命周期事件的统一入口。
//!
//! 所有挂钩行为由 hooks.toml 声明（见 config.rs），本模块负责：
//! - 构建事件 payload（JSON，command 挂钩经 stdin 收到）；
//! - 按事件取分组、做工具名匹配；
//! - 顺序执行组内处理器，聚合为 HookOutcome；
//! - 任一处理器产生拦截（override_error / prevent_continuation）即短路。

use serde_json::{json, Value};
use tauri::AppHandle;

use super::command::{apply_blocked, context_message_from_hook, run_command_handler, CommandHookResult};
use super::config::{load_hooks_file, HookHandlerConfig, MatcherGroup};
use super::shared::{has_exact_user_message, latest_assistant_text};
use super::types::HookOutcome;
use crate::llm::types::{Content, Message};

/// 工具名匹配：大小写不敏感，支持 `*` 通配符；缺省/空/`*` 匹配一切。
fn matcher_matches(matcher: Option<&str>, tool_name: &str) -> bool {
    let Some(pattern) = matcher.map(|m| m.trim()).filter(|m| !m.is_empty()) else {
        return true;
    };
    let pattern = pattern.to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
    let name = tool_name.to_ascii_lowercase();
    wildcard_match(&pattern, &name)
}

/// 简易 `*` 通配匹配：把模式按 `*` 切段后顺序匹配。
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let segments: Vec<&str> = pattern.split('*').collect();
    if segments.len() == 1 {
        return pattern == text;
    }

    let mut rest = text;
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        let first = index == 0;
        let last = index == segments.len() - 1;
        match rest.find(segment) {
            Some(pos) => {
                if first && pos != 0 {
                    return false;
                }
                if last && pos + segment.len() != rest.len() {
                    return false;
                }
                rest = &rest[pos + segment.len()..];
            }
            None => return false,
        }
    }
    true
}

fn substitute_placeholders(text: &str, payload: &Value) -> String {
    let lookup = |key: &str| -> String {
        payload
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    text.replace("{tool_name}", &lookup("tool_name"))
        .replace("{conversation_id}", &lookup("conversation_id"))
        .replace("{subagent_name}", &lookup("subagent_name"))
        .replace("{stop_reason}", &lookup("stop_reason"))
        .replace("{error}", &lookup("error"))
}

/// 单事件分发。`tool_name` 仅工具类事件用于匹配；`messages` 仅 Stop 事件用于
/// 助手消息计数与上下文去重。
async fn dispatch(
    event_name: &'static str,
    groups: Vec<MatcherGroup>,
    payload: Value,
    tool_name: Option<&str>,
    messages: Option<&[Message]>,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    if groups.is_empty() {
        return outcome;
    }

    for group in groups {
        if let Some(name) = tool_name {
            if !matcher_matches(group.matcher.as_deref(), name) {
                continue;
            }
        }

        for handler in &group.hooks {
            if outcome.override_error.is_some() || outcome.prevent_continuation {
                break;
            }
            apply_handler(event_name, handler, &payload, messages, &mut outcome).await;
        }

        if outcome.override_error.is_some() || outcome.prevent_continuation {
            break;
        }
    }

    outcome
}

async fn apply_handler(
    event_name: &'static str,
    handler: &HookHandlerConfig,
    payload: &Value,
    messages: Option<&[Message]>,
    outcome: &mut HookOutcome,
) {
    match handler {
        HookHandlerConfig::Command { .. } => {
            match run_command_handler(handler, event_name, payload).await {
                CommandHookResult::Passed { context: Some(text) } => {
                    outcome
                        .additional_messages
                        .push(context_message_from_hook(&format!("[{}] {}", event_name, text)));
                }
                CommandHookResult::Blocked { reason } => {
                    apply_blocked(outcome, event_name, reason);
                }
                CommandHookResult::Passed { context: None } | CommandHookResult::Ignored => {}
            }
        }

        HookHandlerConfig::Context { text } => {
            let rendered = substitute_placeholders(text, payload);
            let message = context_message_from_hook(&format!("[{}] {}", event_name, rendered));
            // Stop 事件去重：静态上下文若已在历史中出现过完全相同的消息则跳过，
            // 避免无限触发续跑循环。
            if event_name == "Stop" {
                if let Some(messages) = messages {
                    if let Content::Text(body) = &message.content {
                        if has_exact_user_message(messages, body) {
                            return;
                        }
                    }
                }
            }
            outcome.additional_messages.push(message);
        }

        HookHandlerConfig::Block { reason } => {
            apply_blocked(outcome, event_name, substitute_placeholders(reason, payload));
        }

        HookHandlerConfig::StopWhen { pattern } => {
            let target = match event_name {
                "PostToolUse" => payload
                    .get("tool_output")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                "Stop" => payload
                    .get("latest_assistant_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                _ => "",
            };
            if !pattern.is_empty() && target.contains(pattern.as_str()) {
                outcome.prevent_continuation = true;
                outcome.stop_reason = Some(format!(
                    "{} hook stopped continuation: matched pattern '{}'",
                    event_name, pattern
                ));
            }
        }

        HookHandlerConfig::StopOnError => {
            if event_name == "PostToolUseFailure" {
                let tool_name = payload.get("tool_name").and_then(|v| v.as_str()).unwrap_or("?");
                let error = payload.get("error").and_then(|v| v.as_str()).unwrap_or_default();
                outcome.prevent_continuation = true;
                outcome.stop_reason = Some(format!(
                    "PostToolUseFailure hook stopped continuation after '{}' failed: {}",
                    tool_name, error
                ));
            }
        }

        HookHandlerConfig::MaxAssistantMessages { limit } => {
            if event_name == "Stop" && *limit > 0 {
                if let Some(messages) = messages {
                    let assistant_count = messages
                        .iter()
                        .filter(|m| m.role == crate::llm::types::Role::Assistant)
                        .count();
                    if assistant_count > *limit {
                        outcome.prevent_continuation = true;
                        outcome.stop_reason = Some(format!(
                            "Stop hook prevented continuation: assistant message count {} exceeds limit {}",
                            assistant_count, limit
                        ));
                    }
                }
            }
        }

        HookHandlerConfig::AppendStopReason { text } => {
            let rendered = substitute_placeholders(text, payload);
            match event_name {
                "SessionEnd" => {
                    let base = payload.get("stop_reason").and_then(|v| v.as_str()).unwrap_or_default();
                    outcome.stop_reason = Some(format!("{} | [SessionEnd] {}", base, rendered));
                }
                "Error" => {
                    let base = payload.get("error").and_then(|v| v.as_str()).unwrap_or_default();
                    outcome.override_error = Some(format!("{} | [ErrorHook] {}", base, rendered));
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 12 个对外入口：签名与调用点一一对应，内部全部走声明式分发。
// ---------------------------------------------------------------------------

async fn dispatch_lifecycle(
    app: &AppHandle,
    event_name: &'static str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file
        .hooks
        .all_groups()
        .into_iter()
        .find(|(name, _)| *name == event_name)
        .map(|(_, groups)| groups.clone())
        .unwrap_or_default();
    let payload = json!({
        "event": event_name,
        "conversation_id": conversation_id.unwrap_or_default(),
    });
    dispatch(event_name, groups, payload, None, None).await
}

pub async fn run_session_start_hooks(app: &AppHandle, conversation_id: Option<&str>) -> HookOutcome {
    dispatch_lifecycle(app, "SessionStart", conversation_id).await
}

pub async fn run_user_prompt_submit_hooks(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> HookOutcome {
    dispatch_lifecycle(app, "UserPromptSubmit", conversation_id).await
}

pub async fn run_pre_compact_hooks(app: &AppHandle, conversation_id: Option<&str>) -> HookOutcome {
    dispatch_lifecycle(app, "PreCompact", conversation_id).await
}

pub async fn run_post_compact_hooks(app: &AppHandle, conversation_id: Option<&str>) -> HookOutcome {
    dispatch_lifecycle(app, "PostCompact", conversation_id).await
}

pub async fn run_session_end_hooks(
    app: &AppHandle,
    stop_reason: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.session_end.clone();
    let payload = json!({
        "event": "SessionEnd",
        "conversation_id": conversation_id.unwrap_or_default(),
        "stop_reason": stop_reason,
    });
    dispatch("SessionEnd", groups, payload, None, None).await
}

pub async fn run_error_hooks(
    app: &AppHandle,
    error: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.error.clone();
    let payload = json!({
        "event": "Error",
        "conversation_id": conversation_id.unwrap_or_default(),
        "error": error,
    });
    dispatch("Error", groups, payload, None, None).await
}

pub async fn run_subagent_start_hooks(
    app: &AppHandle,
    subagent_name: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.subagent_start.clone();
    let payload = json!({
        "event": "SubagentStart",
        "conversation_id": conversation_id.unwrap_or_default(),
        "subagent_name": subagent_name,
    });
    dispatch("SubagentStart", groups, payload, None, None).await
}

pub async fn run_subagent_stop_hooks(
    app: &AppHandle,
    subagent_name: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.subagent_stop.clone();
    let payload = json!({
        "event": "SubagentStop",
        "conversation_id": conversation_id.unwrap_or_default(),
        "subagent_name": subagent_name,
    });
    dispatch("SubagentStop", groups, payload, None, None).await
}

pub async fn run_pre_tool_use_hooks(
    app: &AppHandle,
    tool_name: &str,
    input: &Value,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.pre_tool_use.clone();
    let payload = json!({
        "event": "PreToolUse",
        "conversation_id": conversation_id.unwrap_or_default(),
        "tool_name": tool_name,
        "tool_input": input,
    });
    dispatch("PreToolUse", groups, payload, Some(tool_name), None).await
}

pub async fn run_post_tool_use_hooks(
    app: &AppHandle,
    tool_name: &str,
    input: &Value,
    output: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.post_tool_use.clone();
    let payload = json!({
        "event": "PostToolUse",
        "conversation_id": conversation_id.unwrap_or_default(),
        "tool_name": tool_name,
        "tool_input": input,
        "tool_output": output,
    });
    dispatch("PostToolUse", groups, payload, Some(tool_name), None).await
}

pub async fn run_post_tool_use_failure_hooks(
    app: &AppHandle,
    tool_name: &str,
    input: &Value,
    error: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.post_tool_use_failure.clone();
    let payload = json!({
        "event": "PostToolUseFailure",
        "conversation_id": conversation_id.unwrap_or_default(),
        "tool_name": tool_name,
        "tool_input": input,
        "error": error,
    });
    dispatch(
        "PostToolUseFailure",
        groups,
        payload,
        Some(tool_name),
        None,
    )
    .await
}

pub async fn run_stop_hooks(
    app: &AppHandle,
    messages: &[Message],
    conversation_id: Option<&str>,
) -> HookOutcome {
    let file = load_hooks_file(app);
    let groups = file.hooks.stop.clone();
    let assistant_count = messages
        .iter()
        .filter(|m| m.role == crate::llm::types::Role::Assistant)
        .count();
    let payload = json!({
        "event": "Stop",
        "conversation_id": conversation_id.unwrap_or_default(),
        "assistant_message_count": assistant_count,
        "latest_assistant_text": latest_assistant_text(messages),
    });
    dispatch("Stop", groups, payload, None, Some(messages)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matcher_case_insensitive_exact() {
        assert!(matcher_matches(Some("Bash"), "bash"));
        assert!(matcher_matches(Some("bash"), "Bash"));
        assert!(!matcher_matches(Some("bash"), "write"));
    }

    #[test]
    fn matcher_wildcard() {
        assert!(matcher_matches(Some("*"), "anything"));
        assert!(matcher_matches(None, "anything"));
        assert!(matcher_matches(Some(""), "anything"));
        assert!(matcher_matches(Some("mcp_*"), "mcp_github"));
        assert!(matcher_matches(Some("*_tool"), "search_tool"));
        assert!(matcher_matches(Some("a*c"), "abc"));
        assert!(!matcher_matches(Some("mcp_*"), "bash"));
        assert!(!matcher_matches(Some("a*c"), "ab"));
    }

    #[test]
    fn placeholders_substituted() {
        let payload = json!({ "tool_name": "Bash", "conversation_id": "c1" });
        let out = substitute_placeholders("tool={tool_name} cid={conversation_id}", &payload);
        assert_eq!(out, "tool=Bash cid=c1");
    }
}
