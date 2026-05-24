use tauri::AppHandle;

use crate::llm::types::{Content, Message, Role};

use super::config::HookConfig;
use super::shared::{has_exact_user_message, latest_assistant_text};
use super::types::HookOutcome;

pub fn run_stop_hooks(
    app: &AppHandle,
    messages: &[Message],
    _conversation_id: Option<&str>,
) -> HookOutcome {
    let config = match HookConfig::from_app(app) {
        Ok(config) => config,
        Err(error) => return HookOutcome::from_error(error),
    };
    let mut out = HookOutcome::default();

    if let Some(limit) = config.value("NOVA_STOP_HOOK_MAX_ASSISTANT_MESSAGES") {
        if let Ok(max_assistant_messages) = limit.parse::<usize>() {
            if max_assistant_messages > 0 {
                let assistant_count = messages
                    .iter()
                    .filter(|m| m.role == Role::Assistant)
                    .count();
                if assistant_count > max_assistant_messages {
                    out.prevent_continuation = true;
                    out.stop_reason = Some(format!(
                        "Stop hook prevented continuation: assistant message count {} exceeds limit {}",
                        assistant_count, max_assistant_messages
                    ));
                    return out;
                }
            }
        }
    }

    if let Some(block_pattern) = config.value("NOVA_STOP_HOOK_BLOCK_PATTERN") {
        let assistant_text = latest_assistant_text(messages);
        if assistant_text.contains(block_pattern) {
            out.prevent_continuation = true;
            out.stop_reason = Some(format!(
                "Stop hook prevented continuation because assistant text matched pattern '{}'",
                block_pattern
            ));
            return out;
        }
    }

    if let Some(extra) = config.value("NOVA_STOP_HOOK_APPEND_CONTEXT") {
        let body = format!("[StopHookContext] {}", extra);
        if !has_exact_user_message(messages, &body) {
            out.additional_messages.push(Message {
                role: Role::User,
                content: Content::Text(body),
            });
        }
    }

    out
}
