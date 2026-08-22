//! 挂钩 command 处理器运行时：外部命令执行、超时、退出码语义。
//!
//! 约定（对齐 Claude Code / codex 生态）：
//! - 事件 JSON 经 stdin 传入；
//! - 退出码 0 = 通过，非空 stdout 作为附加上下文注入；
//! - 退出码 2 = 拦截，stderr 作为拦截原因；
//! - 其他非零 = 记录警告并忽略（不阻塞会话）；
//! - 超时视为拦截。

use serde_json::Value;

use super::config::HookHandlerConfig;
use super::types::HookOutcome;
use crate::llm::types::{Content, Message, Role};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

pub(crate) enum CommandHookResult {
    /// 退出码 0：可选附加上下文。
    Passed { context: Option<String> },
    /// 退出码 2 或超时：拦截，附带原因。
    Blocked { reason: String },
    /// 其他非零退出：忽略。
    Ignored,
}

fn shell_command_line(command: &str, command_windows: Option<&str>) -> (std::ffi::OsString, String) {
    if cfg!(windows) {
        let line = command_windows.filter(|s| !s.trim().is_empty()).unwrap_or(command);
        (std::ffi::OsString::from("cmd"), format!("/c {}", line))
    } else {
        (std::ffi::OsString::from("sh"), format!("-c {}", command))
    }
}

/// 执行一个 command 挂钩。async=true 时发射后立即返回（不影响结果）。
pub(crate) async fn run_command_handler(
    handler: &HookHandlerConfig,
    event_name: &str,
    payload: &Value,
) -> CommandHookResult {
    let HookHandlerConfig::Command {
        command,
        command_windows,
        timeout_sec,
        fire_and_forget,
    } = handler
    else {
        return CommandHookResult::Ignored;
    };

    if command.trim().is_empty() {
        tracing::warn!(event = %event_name, "hook command is empty; skipped");
        return CommandHookResult::Ignored;
    }

    let (program, arg_line) = shell_command_line(command, command_windows.as_deref());
    let payload_json = serde_json::to_string(payload).unwrap_or_default();
    let event = event_name.to_string();

    if *fire_and_forget {
        // 异步挂钩：发射后不等待、不采集结果。失败仅记录日志。
        tokio::spawn(async move {
            let spawned = tokio::process::Command::new(&program)
                .arg(&arg_line)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .env("NOVA_HOOK_EVENT", &event)
                .spawn();
            match spawned {
                Ok(mut child) => {
                    let _ = child.wait().await;
                }
                Err(error) => {
                    tracing::warn!(event = %event, error = %error, "async hook spawn failed");
                }
            }
        });
        return CommandHookResult::Ignored;
    }

    let timeout_secs = timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SECS).max(1);

    let spawned = tokio::process::Command::new(&program)
        .arg(&arg_line)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // 超时后 wait future 被 drop，子进程随之被杀。
        .kill_on_drop(true)
        .env("NOVA_HOOK_EVENT", &event)
        .spawn();

    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return CommandHookResult::Blocked {
                reason: format!("Hook command failed to start: {}", error),
            };
        }
    };

    // 事件 JSON 写入 stdin 后立即关闭，避免子进程读取阻塞。
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(payload_json.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait_with_output()).await;

    match output {
        Ok(Ok(output)) => {
            let code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if code == 0 {
                CommandHookResult::Passed {
                    context: if stdout.is_empty() { None } else { Some(stdout) },
                }
            } else if code == 2 {
                let reason = if stderr.is_empty() {
                    format!("Blocked by hook command (event {})", event)
                } else {
                    stderr
                };
                CommandHookResult::Blocked { reason }
            } else {
                tracing::warn!(
                    event = %event,
                    code = code,
                    stderr = %stderr,
                    "hook command exited with non-zero/non-2 code; ignored"
                );
                CommandHookResult::Ignored
            }
        }
        Ok(Err(error)) => CommandHookResult::Blocked {
            reason: format!("Hook command failed: {}", error),
        },
        Err(_) => {
            // timeout 已将 wait future（连同 child）drop，kill_on_drop 负责杀进程。
            CommandHookResult::Blocked {
                reason: format!("Hook command timed out after {}s", timeout_secs),
            }
        }
    }
}

/// 把 command 结果折叠进 HookOutcome。
/// exit 2 / 超时语义按事件位置区分：工具执行前事件拦截工具（override_error），
/// 其余事件终止续跑（prevent_continuation）。
pub(crate) fn apply_blocked(outcome: &mut HookOutcome, event_name: &str, reason: String) {
    if matches!(event_name, "PreToolUse") {
        outcome.override_error = Some(format!("Blocked by hook: {}", reason));
    } else {
        outcome.prevent_continuation = true;
        outcome.stop_reason = Some(format!("Stopped by hook: {}", reason));
    }
}

pub(crate) fn context_message_from_hook(text: &str) -> Message {
    Message {
        role: Role::User,
        content: Content::Text(text.trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_pre_tool_use_sets_override_error() {
        let mut outcome = HookOutcome::default();
        apply_blocked(&mut outcome, "PreToolUse", "deny".into());
        assert!(outcome.override_error.is_some());
        assert!(!outcome.prevent_continuation);
    }

    #[test]
    fn blocked_other_events_stops_continuation() {
        let mut outcome = HookOutcome::default();
        apply_blocked(&mut outcome, "PostToolUse", "fatal".into());
        assert!(outcome.prevent_continuation);
        assert!(outcome.override_error.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn shell_line_prefers_windows_variant() {
        let (program, line) = shell_command_line("sh a.sh", Some("pwsh a.ps1"));
        assert_eq!(program, std::ffi::OsString::from("cmd"));
        assert_eq!(line, "/c pwsh a.ps1");
    }
}
