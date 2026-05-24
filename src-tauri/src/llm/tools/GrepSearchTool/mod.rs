use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::AppHandle;

const MAX_OUTPUT_CHARS: usize = 10_000;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_sync_stub, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "grep_search".into(),
        description: "Search for literal text in files inside the conversation WorkspaceRoot.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "The literal text to search for" },
                "path": { "type": "string", "description": "Optional workspace-relative directory or file to search. Defaults to WorkspaceRoot." }
            },
            "required": ["pattern"]
        }),
    }
}

pub fn execute_sync_stub(_input: Value) -> String {
    json!({
        "ok": false,
        "error": "grep_search requires AppHandle-aware execution inside a conversation WorkspaceRoot."
    })
    .to_string()
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

fn push_match(
    workspace_root: &Path,
    file: &Path,
    line_number: usize,
    line: &str,
    output: &mut String,
) {
    if output.len() >= MAX_OUTPUT_CHARS {
        return;
    }
    let path = file
        .strip_prefix(workspace_root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");
    output.push_str(&format!("{}:{}:{}\n", path, line_number, line));
}

fn search_file(workspace_root: &Path, file: &Path, pattern: &str, output: &mut String) {
    if output.len() >= MAX_OUTPUT_CHARS {
        return;
    }
    let Ok(content) = fs::read_to_string(file) else {
        return;
    };
    for (index, line) in content.lines().enumerate() {
        if line.contains(pattern) {
            push_match(workspace_root, file, index + 1, line, output);
            if output.len() >= MAX_OUTPUT_CHARS {
                output.push_str("...\n(Result truncated)");
                return;
            }
        }
    }
}

fn walk(workspace_root: &Path, current: &Path, pattern: &str, output: &mut String) {
    if output.len() >= MAX_OUTPUT_CHARS {
        return;
    }
    if current.is_file() {
        search_file(workspace_root, current, pattern, output);
        return;
    }

    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(workspace_root, &path, pattern, output);
        } else if path.is_file() {
            search_file(workspace_root, &path, pattern, output);
        }
        if output.len() >= MAX_OUTPUT_CHARS {
            break;
        }
    }
}

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    let pattern = match input.get("pattern").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value,
        _ => return json!({ "ok": false, "error": "Missing 'pattern' argument" }).to_string(),
    };
    let path_arg = input
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");

    let workspace_root =
        match crate::command::workspace::workspace_root_for_conversation(app, conversation_id) {
            Ok(root) => root,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
    let target =
        match crate::llm::services::file_changes::resolve_tool_path(&workspace_root, path_arg) {
            Ok(path) => path,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
    if !target.exists() {
        return json!({ "ok": false, "error": "Search path does not exist" }).to_string();
    }

    let mut output = String::new();
    walk(&workspace_root, &target, pattern, &mut output);
    if output.is_empty() {
        "No matches found".into()
    } else {
        output
    }
}
