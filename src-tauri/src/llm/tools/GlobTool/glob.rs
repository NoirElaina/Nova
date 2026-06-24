use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::SystemTime;
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "Glob".into(),
        description: r#"Fast file pattern matching. Supports glob patterns like `**/*.js` or `src/**/*.ts`.

- `pattern`: the glob pattern to match files against.
- `path`: the directory to search in. Defaults to the current workspace directory if not specified.

Returns matching file paths sorted by modification time (most recent first)."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. If not specified, the current working directory will be used."
                }
            },
            "required": ["pattern"]
        }),
    }
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let raw_pattern = input
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: pattern"))?;

    let base_path = match input.get("path").and_then(Value::as_str) {
        Some(p) => PathBuf::from(p),
        None => crate::command::workspace::workspace_root_for_conversation(app, conversation_id)
            .unwrap_or_else(|_| {
                crate::command::workspace::default_workspace_root(app)
                    .unwrap_or_else(|_| PathBuf::from("."))
            }),
    };

    if !base_path.is_dir() {
        return Err(ToolFailure::new(format!(
            "path is not a directory or does not exist: {}",
            base_path.display()
        )));
    }

    let search_pattern = base_path.join(raw_pattern);
    let pattern_str = search_pattern.to_string_lossy();

    let entries = match glob::glob(&pattern_str) {
        Ok(iter) => iter,
        Err(e) => {
            return Err(ToolFailure::invalid_input(format!("Invalid glob pattern: {}", e)));
        }
    };

    let mut results: Vec<(PathBuf, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        if entry.is_file() {
            let mtime = std::fs::metadata(&entry)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            results.push((entry, mtime));
        }
    }

    results.sort_by(|a, b| b.1.cmp(&a.1));

    if results.is_empty() {
        Ok(ToolOutcome::text(format!(
            "No files matched pattern: {}",
            raw_pattern
        )))
    } else {
        let paths: Vec<String> = results
            .into_iter()
            .map(|(path, _)| path.display().to_string())
            .collect();
        Ok(ToolOutcome::text(paths.join("\n")))
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
