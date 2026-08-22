use crate::llm::services::shell_sessions::ShellExecutionResult;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission), ToolDisclosure::Core)
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    crate::llm::utils::permissions::describe_shell_command_permission(
        "Bash",
        "终端命令",
        input,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "Bash".into(),
        description: r#"Executes a shell command in a conversation-scoped persistent shell session. The working directory and environment persist between calls, so `cd` and exported variables carry over.

**Platform**: PowerShell 7 (pwsh) on Windows; sh on Linux/macOS. Write commands for the current platform — do not assume bash syntax on Windows.

## When to use this tool
- Running builds, tests, linters, type checkers, package managers, and git commands.
- System operations with no dedicated tool (process/environment inspection, file permissions).
- NOT for reading files (use Read), searching content (use Grep), or finding files by name (use Glob) — dedicated tools are structured, permission-aware, and cheaper.

## Parameters
- `command` (required): the command to execute. Prefer focused single commands; chain only trivial sequences.
- `description`: a short (3-5 word) active-voice summary shown to the user.
- `timeout`: milliseconds, default 120000 (2 min), max 1800000 (30 min). Long builds and test suites need an explicit larger timeout.
- `run_in_background`: keep the session usable while the command runs — use for dev servers and file watchers.

## Output and failure semantics
- The result includes exit code, stdout, stderr, and the final working directory. A non-zero exit is a failure — read stderr before retrying; do not blindly re-run the same command.
- Output is capped at 30000 characters per stream (stdout/stderr); excess is truncated with a marker. For huge output, redirect to a file and inspect it with Read/Grep.
- Commands that appear to wait on an interactive prompt (e.g. `(y/n)`, `Continue?`) are aborted — retry with piped input (`echo y | command`) or a non-interactive flag (`-y`, `--yes`).
- Interactive TUI programs (vim, top, REPLs) are not supported; use non-interactive modes.

## Common mistakes
- Writing files via shell redirects (Out-File, Set-Content, echo >, here-strings) — they introduce BOM/CRLF encoding problems; use Write/Edit instead.
- Long-running foreground commands without a raised timeout.
- Assuming Unix syntax on Windows (pwsh uses `$env:VAR`, `Get-Content`, backtick escapes).
- Reading files with cat/Get-Content instead of the Read tool."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does in active voice"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds (max 1800000)"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background."
                }
            },
            "required": ["command"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let cmd = match input.get("command").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.to_string(),
        _ => return Err(ToolFailure::invalid_input("Missing 'command' argument")),
    };
    let timeout_ms = input
        .get("timeout")
        .and_then(|value| value.as_u64());
    let background = input
        .get("run_in_background")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let workspace_root =
        match crate::command::workspace::workspace_root_string_for_conversation(app, conversation_id)
        {
            Ok(root) => root,
            Err(error) => {
                return Err(ToolFailure::new(format!(
                    "Failed to resolve workspace: {}",
                    error
                )));
            }
        };

    let result = if background {
        crate::llm::services::shell_sessions::run_background(
            conversation_id,
            &cmd,
            Some(&workspace_root),
        )
        .await
    } else {
        crate::llm::services::shell_sessions::run_foreground(
            conversation_id,
            &cmd,
            timeout_ms,
            Some(&workspace_root),
        )
        .await
    };

    match result {
        Ok(result) if result.cancelled => Err(ToolFailure::cancelled(shell_failure_text(
            "command cancelled",
            &result,
        ))),
        Ok(result) if result.timed_out => Err(ToolFailure::new(shell_failure_text(
            "command timed out",
            &result,
        ))),
        Ok(result) => Ok(ToolOutcome::json(shell_result_json(result))),
        Err(error) => Err(ToolFailure::new(format!(
            "Failed to execute command: {}",
            error
        ))),
    }
}

fn shell_result_json(result: ShellExecutionResult) -> Value {
    json!({
        "ok": result.exit_code.unwrap_or(1) == 0,
        "stdout": truncate_output(&result.stdout),
        "stderr": truncate_output(&result.stderr),
        "exitCode": result.exit_code,
        "cwd": result.cwd,
        "timedOut": result.timed_out,
        "background": result.background,
        "pid": result.pid
    })
}

fn shell_failure_text(reason: &str, result: &ShellExecutionResult) -> String {
    format!(
        "{reason}\nexitCode: {:?}\ncwd: {}\ntimedOut: {}\nbackground: {}\npid: {:?}\nstdout:\n{}\nstderr:\n{}",
        result.exit_code,
        result.cwd.as_deref().unwrap_or(""),
        result.timed_out,
        result.background,
        result.pid,
        truncate_output(&result.stdout),
        truncate_output(&result.stderr)
    )
}

// 单段输出的字符上限。
// 超出则保留头部并追加截断标记，避免大型命令输出灌满上下文、触发 prompt_too_long。
const MAX_OUTPUT_CHARS: usize = 30_000;

/// 超长输出截断：保头部 2/3 + 尾部 1/3。
/// 编译/测试的报错信息几乎总在输出尾部，只保头部会让模型“看不见错误”
/// 而盲目重试；尾部必须保留。
fn truncate_output(content: &str) -> String {
    if content.len() <= MAX_OUTPUT_CHARS {
        return content.to_string();
    }

    let head_budget = MAX_OUTPUT_CHARS * 2 / 3;
    let tail_budget = MAX_OUTPUT_CHARS - head_budget;

    // 头部：从预算位置往前退到合法 UTF-8 字符边界。
    let mut head_cut = head_budget;
    while head_cut > 0 && !content.is_char_boundary(head_cut) {
        head_cut -= 1;
    }
    // 尾部：从预算位置往后推到合法 UTF-8 字符边界。
    // len > MAX_OUTPUT_CHARS = head_budget + tail_budget 保证 tail_start > head_cut。
    let mut tail_start = content.len() - tail_budget;
    while tail_start < content.len() && !content.is_char_boundary(tail_start) {
        tail_start += 1;
    }

    let omitted = &content[head_cut..tail_start];
    let omitted_lines = omitted.matches('\n').count();
    format!(
        "{}\n\n... [middle omitted: ~{} chars / {} lines] ...\n\n{}",
        &content[..head_cut],
        omitted.len(),
        omitted_lines,
        &content[tail_start..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_unchanged() {
        assert_eq!(truncate_output("hello"), "hello");
    }

    #[test]
    fn long_output_keeps_head_and_tail() {
        // 40000 行号行，超出 30000 上限。
        let content = (0..40_000).map(|i| format!("line{}", i)).collect::<Vec<_>>().join("\n");
        let out = truncate_output(&content);
        // 头部保留：第一行还在。
        assert!(out.starts_with("line0"));
        // 尾部保留：最后一行还在（旧实现会把它截丢）。
        assert!(out.contains("line39999"));
        // 中间省略标记存在。
        assert!(out.contains("[middle omitted:"));
        // 总长受控（≈上限 + 标记开销）。
        assert!(out.len() <= MAX_OUTPUT_CHARS + 200);
    }

    #[test]
    fn truncation_is_multibyte_safe() {
        // 中文 3 字节/字符：构造超限内容，截断后仍是合法 UTF-8。
        let content = "错".repeat(20_000);
        let out = truncate_output(&content);
        assert!(out.chars().count() <= MAX_OUTPUT_CHARS + 64);
    }
}
