//! 挂钩事件分发：5 个生命周期与工具事件的统一入口。
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
use super::types::{HookEvent, HookOutcome};

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
        .replace("{error}", &lookup("error"))
}

/// 单事件分发。`tool_name` 仅工具类事件用于匹配。
async fn dispatch(
    event_name: &'static str,
    groups: Vec<MatcherGroup>,
    payload: Value,
    tool_name: Option<&str>,
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
            apply_handler(event_name, handler, &payload, &mut outcome).await;
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
    }
}

// ---------------------------------------------------------------------------
// 统一事件入口（观察者模式）：调用点只发射 HookEvent，本函数按事件名
// 查 hooks.toml 分组并分发。
// ---------------------------------------------------------------------------

/// 事件 → (事件名, payload, 工具名) 的分发参数。
fn event_into_parts(event: HookEvent) -> (&'static str, Value, Option<String>) {
    match event {
        HookEvent::SubagentStart { conversation_id, subagent_name } => (
            "SubagentStart",
            json!({
                "event": "SubagentStart",
                "conversation_id": conversation_id.unwrap_or_default(),
                "subagent_name": subagent_name,
            }),
            None,
        ),
        HookEvent::SubagentStop { conversation_id, subagent_name } => (
            "SubagentStop",
            json!({
                "event": "SubagentStop",
                "conversation_id": conversation_id.unwrap_or_default(),
                "subagent_name": subagent_name,
            }),
            None,
        ),
        HookEvent::PreToolUse { conversation_id, tool_name, input } => (
            "PreToolUse",
            json!({
                "event": "PreToolUse",
                "conversation_id": conversation_id.unwrap_or_default(),
                "tool_name": tool_name,
                "tool_input": input,
            }),
            Some(tool_name),
        ),
        HookEvent::PostToolUse { conversation_id, tool_name, input, output } => (
            "PostToolUse",
            json!({
                "event": "PostToolUse",
                "conversation_id": conversation_id.unwrap_or_default(),
                "tool_name": tool_name,
                "tool_input": input,
                "tool_output": output,
            }),
            Some(tool_name),
        ),
        HookEvent::PostToolUseFailure { conversation_id, tool_name, input, error } => (
            "PostToolUseFailure",
            json!({
                "event": "PostToolUseFailure",
                "conversation_id": conversation_id.unwrap_or_default(),
                "tool_name": tool_name,
                "tool_input": input,
                "error": error,
            }),
            Some(tool_name),
        ),
    }
}

/// 挂钩事件唯一发射入口：按事件名从 hooks.toml 取分组并分发。
pub async fn emit_hook_event(app: &AppHandle, event: HookEvent) -> HookOutcome {
    let (event_name, payload, tool_name) = event_into_parts(event);
    let file = load_hooks_file(app);
    let groups = file.hooks.groups_for(event_name);
    dispatch(event_name, groups, payload, tool_name.as_deref()).await
}

// ---------------------------------------------------------------------------
// 封装的挂钩运行入口
// ---------------------------------------------------------------------------

pub async fn run_subagent_start_hooks(
    app: &AppHandle,
    subagent_name: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    emit_hook_event(
        app,
        HookEvent::SubagentStart {
            conversation_id: conversation_id.map(str::to_string),
            subagent_name: subagent_name.to_string(),
        },
    )
    .await
}

pub async fn run_subagent_stop_hooks(
    app: &AppHandle,
    subagent_name: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    emit_hook_event(
        app,
        HookEvent::SubagentStop {
            conversation_id: conversation_id.map(str::to_string),
            subagent_name: subagent_name.to_string(),
        },
    )
    .await
}

pub async fn run_pre_tool_use_hooks(
    app: &AppHandle,
    tool_name: &str,
    input: &Value,
    conversation_id: Option<&str>,
) -> HookOutcome {
    emit_hook_event(
        app,
        HookEvent::PreToolUse {
            conversation_id: conversation_id.map(str::to_string),
            tool_name: tool_name.to_string(),
            input: input.clone(),
        },
    )
    .await
}

pub async fn run_post_tool_use_hooks(
    app: &AppHandle,
    tool_name: &str,
    input: &Value,
    output: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    emit_hook_event(
        app,
        HookEvent::PostToolUse {
            conversation_id: conversation_id.map(str::to_string),
            tool_name: tool_name.to_string(),
            input: input.clone(),
            output: output.to_string(),
        },
    )
    .await
}

pub async fn run_post_tool_use_failure_hooks(
    app: &AppHandle,
    tool_name: &str,
    input: &Value,
    error: &str,
    conversation_id: Option<&str>,
) -> HookOutcome {
    emit_hook_event(
        app,
        HookEvent::PostToolUseFailure {
            conversation_id: conversation_id.map(str::to_string),
            tool_name: tool_name.to_string(),
            input: input.clone(),
            error: error.to_string(),
        },
    )
    .await
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
