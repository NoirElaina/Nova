use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use tauri::AppHandle;
use tauri::Manager;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "Grep".into(),
        description: r#"Content search built on ripgrep (rg). Prefer this over shell grep/rg.

- `pattern`: a regular expression to search for in file contents.
- `path`: file or directory to search in. Defaults to the workspace root if not specified.
- `glob`: optional file filter pattern (e.g. `"*.rs"`, `"*.{ts,tsx}"`).
- `output_mode`: `"content"` (matching lines with line numbers), `"files_with_matches"` (file paths only, default), or `"count"` (match counts per file).
- `-A`: number of lines to show after each match (rg -A).
- `-B`: number of lines to show before each match (rg -B).
- `-C`: number of lines to show before and after each match (rg -C).
- `-i`: set to true for case-insensitive search.
- `-n`: show line numbers in output (rg -n). Defaults to true when output_mode is "content".
- `multiline`: enable multiline mode where `.` matches newlines and patterns can span lines (rg -U --multiline-dotall).
- `head_limit`: limit output to the first N lines/entries. Defaults to 250.
- `offset`: skip first N lines/entries before applying head_limit."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regular expression pattern to search for in file contents"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in. Defaults to the workspace root."
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. \"*.rs\", \"*.{ts,tsx}\")"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode. Defaults to \"files_with_matches\"."
                },
                "-A": {
                    "type": "integer",
                    "description": "Number of lines to show after each match (rg -A). Requires output_mode: \"content\"."
                },
                "-B": {
                    "type": "integer",
                    "description": "Number of lines to show before each match (rg -B). Requires output_mode: \"content\"."
                },
                "-C": {
                    "type": "integer",
                    "description": "Number of lines to show before and after each match (rg -C). Requires output_mode: \"content\"."
                },
                "-i": {
                    "type": "boolean",
                    "description": "Case insensitive search (rg -i)"
                },
                "-n": {
                    "type": "boolean",
                    "description": "Show line numbers in output (rg -n). Requires output_mode: \"content\". Defaults to true."
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multiline mode where . matches newlines and patterns can span lines (rg -U --multiline-dotall). Default: false."
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Limit output to the first N lines/entries. Defaults to 250."
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip first N lines/entries before applying head_limit. Defaults to 0."
                }
            },
            "required": ["pattern"]
        }),
    }
}

const DEFAULT_HEAD_LIMIT: usize = 250;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;

fn find_rg_path(app: &AppHandle) -> String {
    if let Ok(val) = std::env::var("NOVA_RG_PATH") {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() && PathBuf::from(&trimmed).exists() {
            return trimmed;
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join("bin").join(
            if cfg!(target_os = "windows") {
                "rg.exe"
            } else {
                "rg"
            },
        );
        if bundled.exists() {
            return bundled.display().to_string();
        }
    }
    "rg".to_string()
}

fn resolve_base_path(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: &Value,
) -> PathBuf {
    match input.get("path").and_then(Value::as_str) {
        Some(p) => PathBuf::from(p),
        None => crate::command::workspace::workspace_root_for_conversation(app, conversation_id)
            .unwrap_or_else(|_| {
                crate::command::workspace::default_workspace_root(app)
                    .unwrap_or_else(|_| PathBuf::from("."))
            }),
    }
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let pattern = input
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: pattern"))?;

    let base_path = resolve_base_path(app, conversation_id, &input);

    let output_mode = input
        .get("output_mode")
        .and_then(Value::as_str)
        .unwrap_or("files_with_matches");

    let case_insensitive = input
        .get("-i")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let show_line_numbers = input
        .get("-n")
        .and_then(Value::as_bool)
        .unwrap_or(output_mode == "content");

    let head_limit = input
        .get("head_limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_HEAD_LIMIT);

    let offset = input
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);

    let context_before = input
        .get("-B")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let context_after = input
        .get("-A")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let context = input
        .get("-C")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let multiline = input
        .get("multiline")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let file_glob = input.get("glob").and_then(Value::as_str);

    let rg_path = find_rg_path(app);

    let mut cmd = Command::new(&rg_path);

    cmd.arg("--no-heading");
    cmd.arg("--with-filename");

    match output_mode {
        "content" => {
            if show_line_numbers {
                cmd.arg("--line-number");
            }
        }
        "count" => {
            cmd.arg("--count");
        }
        _ => {
            cmd.arg("--files-with-matches");
        }
    }

    if case_insensitive {
        cmd.arg("-i");
    }

    if let Some(n) = context {
        cmd.arg("-C");
        cmd.arg(n.to_string());
    }
    if let Some(n) = context_before {
        cmd.arg("-B");
        cmd.arg(n.to_string());
    }
    if let Some(n) = context_after {
        cmd.arg("-A");
        cmd.arg(n.to_string());
    }

    if multiline {
        cmd.arg("-U");
        cmd.arg("--multiline-dotall");
    }

    if let Some(g) = file_glob {
        cmd.arg("--glob");
        cmd.arg(g);
    }

    cmd.arg("--");
    cmd.arg(pattern);
    cmd.arg(&base_path);

    let output = cmd.output().map_err(|e| {
        ToolFailure::new(format!("Failed to run rg: {}", e))
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.code() == Some(2) {
        return Err(ToolFailure::new(format!(
            "rg error: {}",
            stderr.trim()
        )));
    }

    if stdout.trim().is_empty() {
        return Ok(ToolOutcome::text("No matches found."));
    }

    let lines: Vec<&str> = stdout.lines().collect();
    let total = lines.len();

    // Apply offset first, then head_limit.
    let after_offset = if offset > 0 && offset < total {
        &lines[offset..]
    } else {
        &lines[..]
    };

    let limited: Vec<&str> = after_offset.iter().take(head_limit).copied().collect();
    let mut result = limited.join("\n");

    if total > head_limit + offset {
        result.push_str(&format!(
            "\n\n... ({} total results, showing {}-{})",
            total,
            offset + 1,
            (offset + limited.len()).min(total)
        ));
    }

    if result.len() > MAX_OUTPUT_BYTES {
        let mut boundary = MAX_OUTPUT_BYTES;
        while !result.is_char_boundary(boundary) {
            boundary -= 1;
        }
        result = format!(
            "{}\n\n... (output truncated at {} bytes)",
            &result[..boundary],
            MAX_OUTPUT_BYTES
        );
    }

    Ok(ToolOutcome::text(result))
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
