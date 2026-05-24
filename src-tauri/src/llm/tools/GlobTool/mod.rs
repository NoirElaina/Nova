use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_sync_stub, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "glob_search".into(),
        description: "Search files by wildcard pattern inside the conversation WorkspaceRoot.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "root": { "type": "string", "description": "Optional workspace-relative directory to search. Defaults to WorkspaceRoot." },
                "pattern": { "type": "string", "description": "Wildcard pattern against workspace-relative paths. Supports * and ?." },
                "max_results": { "type": "integer", "description": "Maximum number of matches" }
            },
            "required": ["pattern"]
        }),
    }
}

pub fn execute_sync_stub(_input: Value) -> String {
    json!({
        "ok": false,
        "error": "glob_search requires AppHandle-aware execution inside a conversation WorkspaceRoot."
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

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut match_j) = (None, 0usize);

    while j < t.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            match_j = j;
        } else if let Some(star_idx) = star {
            i = star_idx + 1;
            match_j += 1;
            j = match_j;
        } else {
            return false;
        }
    }

    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}

fn walk(workspace_root: &Path, current: &Path, pattern: &str, out: &mut Vec<String>, max: usize) {
    if out.len() >= max {
        return;
    }

    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if out.len() >= max {
            break;
        }

        let path = entry.path();
        if path.is_dir() {
            walk(workspace_root, &path, pattern, out, max);
            continue;
        }

        if let Ok(relative) = path.strip_prefix(workspace_root) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            if wildcard_match(pattern, &relative) {
                out.push(relative);
            }
        }
    }
}

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    let root_arg = input
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let pattern = match input.get("pattern").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'pattern' argument" }).to_string(),
    };
    let max_results = input
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(200)
        .max(1)
        .min(2000);

    let workspace_root =
        match crate::command::workspace::workspace_root_for_conversation(app, conversation_id) {
            Ok(root) => root,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
    let search_root =
        match crate::llm::services::file_changes::resolve_tool_path(&workspace_root, root_arg) {
            Ok(path) => path,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
    if !search_root.exists() {
        return json!({ "ok": false, "error": "Root path does not exist" }).to_string();
    }
    if !search_root.is_dir() {
        return json!({ "ok": false, "error": "Root path is not a directory" }).to_string();
    }

    let mut out = Vec::new();
    walk(
        &workspace_root,
        &search_root,
        pattern,
        &mut out,
        max_results,
    );

    if out.is_empty() {
        "No files matched the pattern".into()
    } else {
        out.join("\n")
    }
}
