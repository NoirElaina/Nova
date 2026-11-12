use serde_json::{json, Value};

use crate::agent::permissions::describe_stdin_permission;
use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;
use crate::services::unified_exec::{
    self, ExecError, SessionId, WriteStdinRequest, DEFAULT_WRITE_STDIN_YIELD_TIME_MS,
    MAX_EMPTY_YIELD_TIME_MS, MAX_YIELD_TIME_MS, MIN_EMPTY_YIELD_TIME_MS, MIN_YIELD_TIME_MS,
};

use super::exec_command::parse_lenient_u64;

pub(super) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        execute_with_app_boxed,
        false,
        Some(permission),
        ToolDisclosure::Core,
    )
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    describe_stdin_permission("write_stdin", input)
}

pub fn tool() -> Tool {
    Tool {
        name: "write_stdin".into(),
        description: format!(
            r#"Writes characters to an active interactive session started by `exec_command`, or polls for new incremental output when `chars` is empty.

- `session_id` (required): Integer session ID returned by `exec_command`.
- `chars`: Text or keystrokes to send to the process stdin/PTY (for example `"y\n"`, `"quit\n"`). Omit or pass `""` to wait and poll new output without sending input.
- Sending `"\u0003"` sends Ctrl+C (`0x03`) to the PTY on `tty=true` sessions (interrupting foreground work while keeping interactive shells alive), or terminates `tty=false` pipe sessions. Only `tty=true` sessions accept arbitrary `chars` input; `tty=false` sessions reject non-empty writes other than `"\u0003"`.
- `yield_time_ms`: Time in milliseconds to wait for output/exit (default {default_yield} ms; clamped to {min_yield}..={max_yield} ms when writing input, or {min_empty}..={max_empty} ms when polling with empty `chars`).
- `max_output_tokens`: Maximum approximate output tokens to return per chunk (default 10000).
- Only returns output generated since the last `exec_command` / `write_stdin` call. When the process exits, `session_id` is omitted and the exit code is reported."#,
            default_yield = DEFAULT_WRITE_STDIN_YIELD_TIME_MS,
            min_yield = MIN_YIELD_TIME_MS,
            max_yield = MAX_YIELD_TIME_MS,
            min_empty = MIN_EMPTY_YIELD_TIME_MS,
            max_empty = MAX_EMPTY_YIELD_TIME_MS,
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "ID of the running exec_command session"
                },
                "chars": {
                    "type": "string",
                    "description": "Bytes/keystrokes to write to stdin (use empty string \"\" to poll output only, \"\\u0003\" for Ctrl+C)"
                },
                "yield_time_ms": {
                    "type": "integer",
                    "description": format!(
                        "Time in milliseconds to wait for output or process exit (non-empty write: {}..={} ms, default {}; empty poll: {}..={} ms)",
                        MIN_YIELD_TIME_MS,
                        MAX_YIELD_TIME_MS,
                        DEFAULT_WRITE_STDIN_YIELD_TIME_MS,
                        MIN_EMPTY_YIELD_TIME_MS,
                        MAX_EMPTY_YIELD_TIME_MS
                    )
                },
                "max_output_tokens": {
                    "type": "integer",
                    "description": "Maximum approximate output tokens to return (default 10000)"
                }
            },
            "required": ["session_id"]
        }),
    }
}

fn parse_lenient_session_id(val: Option<&Value>) -> Option<SessionId> {
    let v = val?;
    if let Some(i) = v.as_i64() {
        return SessionId::try_from(i).ok();
    }
    if let Some(u) = v.as_u64() {
        return SessionId::try_from(u).ok();
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 1.0 && f.fract() == 0.0 && f <= (SessionId::MAX as f64) {
            return Some(f as SessionId);
        }
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse::<SessionId>().ok();
    }
    None
}

async fn execute_async(
    _core: &Core,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let session_id = parse_lenient_session_id(input.get("session_id")).ok_or_else(|| {
        ToolFailure::invalid_input("Missing or invalid integer parameter: session_id")
    })?;
    let chars = input
        .get("chars")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let yield_time_ms = parse_lenient_u64(input.get("yield_time_ms"));
    let max_output_tokens = parse_lenient_u64(input.get("max_output_tokens"))
        .and_then(|v| usize::try_from(v).ok());

    let cancel_token = Some(crate::agent::cancellation::get_token(conversation_id));

    let req = WriteStdinRequest {
        conversation_id: conversation_id.map(str::to_string),
        session_id,
        chars,
        yield_time_ms,
        max_output_tokens,
    };

    match unified_exec::write_stdin(req, cancel_token.as_ref()).await {
        Ok(resp) => Ok(ToolOutcome::text(resp.format_text())),
        Err(ExecError::Cancelled) => Err(ToolFailure::cancelled("write_stdin wait cancelled")),
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
    fn stdin_permission_separates_poll_interrupt_and_input() {
        use crate::agent::permissions::RiskLevel;

        let poll = permission(&json!({ "session_id": 3, "chars": "" })).unwrap();
        assert_eq!(poll.risk, RiskLevel::Safe);

        let interrupt = permission(&json!({ "session_id": 3, "chars": "\u{3}" })).unwrap();
        assert_eq!(interrupt.risk, RiskLevel::Safe);

        let injected = permission(&json!({ "session_id": 3, "chars": "rm -rf build\n" })).unwrap();
        assert_eq!(injected.risk, RiskLevel::Risky);
        assert!(injected.signature.starts_with("write_stdin:3:input:"));
        assert!(injected.signature.contains("rm -rf build"));
    }

    #[test]
    fn parses_session_id_across_non_openai_number_encodings() {
        assert_eq!(parse_lenient_session_id(Some(&json!(3))), Some(3));
        assert_eq!(parse_lenient_session_id(Some(&json!(3.0))), Some(3));
        assert_eq!(parse_lenient_session_id(Some(&json!("3"))), Some(3));
        assert_eq!(parse_lenient_session_id(Some(&json!(3.5))), None);
        assert_eq!(parse_lenient_session_id(Some(&json!(null))), None);
    }
}
