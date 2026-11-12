use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::agent::permissions::{
    describe_shell_command_permission, effective_approval_policy, request_sandbox_retry,
    ApprovalPolicy,
};
use crate::agent::sandbox::SandboxPolicy;
use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;
use crate::services::unified_exec::{
    self, ExecCommandRequest, ExecError, ExecSandboxConfig, DEFAULT_YIELD_TIME_MS,
    MAX_YIELD_TIME_MS, MIN_YIELD_TIME_MS, WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS,
};

pub(super) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        execute_with_app_boxed,
        false,
        Some(permission),
        ToolDisclosure::Core,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "exec_command".into(),
        description: format!(
            r#"Runs a command in a PTY (or pipe when `tty` is false), returning output or yielding an interactive session ID for follow-up input via `write_stdin`.

- **Default workflow**: Long-running servers, REPLs, builds, or interactive prompts automatically yield a `session_id` after `yield_time_ms` (default {default_yield} ms, clamped to {min_yield}..={max_yield} ms, or at least {win_floor} ms on Windows). Pass that `session_id` to `write_stdin` to send keystrokes/newlines (`chars`, requires `tty: true`) or poll new output (`chars: ""`).
- **One-shot commands**: Commands that exit before `yield_time_ms` returns their exit code immediately with no `session_id`.
- **Platform**: PowerShell (pwsh) on Windows by default; user shell / sh on Unix.
- **Parameters**:
  - `cmd` (required): Shell command to execute.
  - `workdir`: Working directory for this process (defaults to conversation workspace root; does NOT persist across separate `exec_command` calls).
  - `shell`: Optional shell override (e.g. `pwsh`, `powershell`, `cmd`, `bash`, `zsh`, `sh`).
  - `login`: Run login shell semantics (`true` by default; adds `-NoProfile` on PowerShell when `false`, or `-lc` vs `-c` on POSIX shells).
  - `tty`: Allocate a real pseudo-terminal (`false` by default). Set `true` for interactive programs/REPLs that need stdin via `write_stdin`.
  - `yield_time_ms`: How long in ms to wait before yielding control while the process runs (default {default_yield}, clamped to {min_yield}..={max_yield}, minimum {win_floor} on Windows).
  - `max_output_tokens`: Maximum approximate output tokens to return per chunk (default 10000).
- Prefer dedicated tools (`Read`, `Write`, `Edit`, `MultiEdit`, `apply_patch`, `Grep`, `Glob`) for reading, editing, or searching files."#,
            default_yield = DEFAULT_YIELD_TIME_MS,
            min_yield = MIN_YIELD_TIME_MS,
            max_yield = MAX_YIELD_TIME_MS,
            win_floor = WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS,
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "cmd": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "workdir": {
                    "type": "string",
                    "description": "Optional working directory (defaults to current workspace root)"
                },
                "shell": {
                    "type": "string",
                    "description": "Optional shell executable or short name (pwsh, powershell, cmd, bash, zsh, sh)"
                },
                "login": {
                    "type": "boolean",
                    "description": "Use login shell initialization when supported (default true)"
                },
                "tty": {
                    "type": "boolean",
                    "description": "Allocate a pseudo-terminal for interactive programs (default false; required to send arbitrary stdin via write_stdin)"
                },
                "yield_time_ms": {
                    "type": "integer",
                    "description": format!(
                        "Time in milliseconds to wait for exit before yielding a running session_id ({}..={}, default {}; minimum {} on Windows)",
                        MIN_YIELD_TIME_MS,
                        MAX_YIELD_TIME_MS,
                        DEFAULT_YIELD_TIME_MS,
                        WINDOWS_INITIAL_EXEC_YIELD_TIME_FLOOR_MS
                    )
                },
                "max_output_tokens": {
                    "type": "integer",
                    "description": "Maximum approximate output tokens to return (default 10000)"
                }
            },
            "required": ["cmd"]
        }),
    }
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    describe_shell_command_permission("exec_command", "执行终端命令", input)
}

pub(super) fn parse_lenient_u64(val: Option<&Value>) -> Option<u64> {
    let v = val?;
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f <= (u64::MAX as f64) {
            return Some(f as u64);
        }
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse::<u64>().ok();
    }
    None
}

fn resolve_workdir(workspace_root: &Path, raw_workdir: Option<&str>) -> Result<PathBuf, ToolFailure> {
    let Some(raw) = raw_workdir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(workspace_root.to_path_buf());
    };
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err(ToolFailure::invalid_input(format!(
            "UNC network paths are not allowed for workdir: {}",
            raw
        )));
    }
    let candidate = Path::new(raw);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    if !resolved.is_dir() {
        return Err(ToolFailure::invalid_input(format!(
            "workdir does not exist or is not a directory: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn should_retry_outside_sandbox(
    policy: SandboxPolicy,
    approval: ApprovalPolicy,
    exit_code: Option<i32>,
) -> bool {
    policy.is_enforcing()
        && approval != ApprovalPolicy::Never
        && exit_code.map(|c| c != 0).unwrap_or(false)
}

async fn execute_async(
    core: &Core,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let cmd = input
        .get("cmd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: cmd"))?
        .to_string();

    let workspace_root =
        crate::services::workspace::workspace_root_for_conversation(core, conversation_id)
            .map_err(|e| ToolFailure::new(format!("Failed to resolve workspace: {e}")))?;
    let workdir = resolve_workdir(
        &workspace_root,
        input.get("workdir").and_then(Value::as_str),
    )?;
    let shell = input
        .get("shell")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let login = input
        .get("login")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let tty = input
        .get("tty")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let yield_time_ms = parse_lenient_u64(input.get("yield_time_ms"));
    let max_output_tokens = parse_lenient_u64(input.get("max_output_tokens"))
        .and_then(|v| usize::try_from(v).ok());

    let cancel_token = Some(crate::agent::cancellation::get_token(conversation_id));
    let sandbox_policy = crate::agent::sandbox::effective_sandbox_policy(core);
    let sandbox_cfg = ExecSandboxConfig {
        policy: sandbox_policy,
        workspace: workspace_root.clone(),
        data_dir: core.config().data_dir(),
    };

    let req = ExecCommandRequest {
        conversation_id: conversation_id.map(str::to_string),
        cmd: cmd.clone(),
        workdir: workdir.clone(),
        shell: shell.clone(),
        login,
        tty,
        yield_time_ms,
        max_output_tokens,
        unsandboxed_override: false,
        sandbox: Some(sandbox_cfg.clone()),
    };

    let result = unified_exec::exec_command(req, cancel_token.as_ref()).await;

    match result {
        Ok(mut resp) => {
            if resp.session_id.is_none() {
                let approval = effective_approval_policy(core);
                if should_retry_outside_sandbox(sandbox_policy, approval, resp.exit_code) {
                    let approved = request_sandbox_retry(core, conversation_id, &cmd)
                        .await
                        .unwrap_or(false);
                    if approved {
                        let retry_req = ExecCommandRequest {
                            conversation_id: conversation_id.map(str::to_string),
                            cmd,
                            workdir,
                            shell,
                            login,
                            tty,
                            yield_time_ms,
                            max_output_tokens,
                            unsandboxed_override: true,
                            sandbox: Some(sandbox_cfg),
                        };
                        if let Ok(mut retried) =
                            unified_exec::exec_command(retry_req, cancel_token.as_ref()).await
                        {
                            retried.sandbox_retried = true;
                            resp = retried;
                        }
                    }
                }
            }
            Ok(ToolOutcome::text(resp.format_text()))
        }
        Err(ExecError::Cancelled) => Err(ToolFailure::cancelled("command cancelled")),
        Err(ExecError::InvalidInput(msg)) => Err(ToolFailure::invalid_input(msg)),
        Err(ExecError::Execution(msg)) => Err(ToolFailure::new(msg)),
    }
}

fn execute_with_app_boxed(
    core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&core, conversation_id.as_deref(), input).await })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_u64_accepts_int_float_and_string() {
        assert_eq!(parse_lenient_u64(Some(&json!(5000))), Some(5000));
        assert_eq!(parse_lenient_u64(Some(&json!(5000.0))), Some(5000));
        assert_eq!(parse_lenient_u64(Some(&json!("5000"))), Some(5000));
        assert_eq!(parse_lenient_u64(Some(&json!(5000.5))), None);
        assert_eq!(parse_lenient_u64(Some(&json!(-1))), None);
    }

    #[test]
    fn resolves_relative_and_rejects_unc_workdir() {
        let root = std::env::temp_dir();
        assert_eq!(resolve_workdir(&root, None).unwrap(), root);
        assert!(resolve_workdir(&root, Some(r"\\fileserver\share")).is_err());
        assert!(resolve_workdir(&root, Some("definitely-does-not-exist-987654")).is_err());
    }
}
