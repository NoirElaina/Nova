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
- `-i`: set to true for case-insensitive search.
- `head_limit`: limit output to the first N lines/entries. Defaults to 250."#
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
                "-i": {
                    "type": "boolean",
                    "description": "Case insensitive search (rg -i)"
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Limit output to the first N lines/entries. Defaults to 250."
                }
            },
            "required": ["pattern"]
        }),
    }
}

const DEFAULT_HEAD_LIMIT: usize = 250;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;

fn find_rg_path(app: &AppHandle) -> String {
    // Try NOVA_RG_PATH env var first.
    if let Ok(val) = std::env::var("NOVA_RG_PATH") {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() && PathBuf::from(&trimmed).exists() {
            return trimmed;
        }
    }
    // Try resource dir bundled rg binary.
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
    // Fallback to "rg" on PATH.
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

    let head_limit = input
        .get("head_limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_HEAD_LIMIT);

    let file_glob = input.get("glob").and_then(Value::as_str);

    let rg_path = find_rg_path(app);

    let mut cmd = Command::new(&rg_path);

    cmd.arg("--no-heading");
    cmd.arg("--with-filename");

    match output_mode {
        "content" => {
            cmd.arg("--line-number");
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
    let limited: Vec<&str> = lines.iter().take(head_limit).copied().collect();
    let mut result = limited.join("\n");

    if total > head_limit {
        result.push_str(&format!(
            "\n\n... ({} total results, showing first {})",
            total, head_limit
        ));
    }

    if result.len() > MAX_OUTPUT_BYTES {
        let truncated = &result[..MAX_OUTPUT_BYTES];
        result = format!(
            "{}\n\n... (output truncated at {} bytes)",
            truncated, MAX_OUTPUT_BYTES
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
