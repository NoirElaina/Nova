use crate::llm::services::search::{find_files, GlobSearchOptions, WalkOptions};
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use crate::llm::utils::paths::absolute_path_from_tool_arg;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "glob_search".into(),
        description: "Search files by glob pattern under an absolute directory path. Uses ripgrep-style ignore handling and glob semantics. Returns absolute file paths.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "root": { "type": "string", "description": "Absolute directory path to search. Relative paths and ~ are rejected." },
                "pattern": { "type": "string", "minLength": 1, "description": "Glob pattern matched against paths relative to root. Supports *, ?, **, character classes, and braces. Bare filename patterns match at any depth." },
                "case_sensitive": { "type": "boolean", "description": "Use case-sensitive glob matching. Defaults to true." },
                "include_hidden": { "type": "boolean", "description": "Include hidden files and directories. Defaults to false." },
                "include_ignored": { "type": "boolean", "description": "Include files ignored by .ignore, .gitignore, .git/info/exclude, and global gitignore. Defaults to false." },
                "follow_symlinks": { "type": "boolean", "description": "Follow symbolic links while walking directories. Defaults to false." },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Maximum number of matches. Defaults to 200." }
            },
            "required": ["root", "pattern"]
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
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let root_arg = input
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolFailure::invalid_input("Missing 'root' argument"))?;
    let pattern = match input.get("pattern").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => return Err(ToolFailure::invalid_input("Missing 'pattern' argument")),
    };
    let max_results = input
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(200)
        .clamp(1, 2000);

    let search_root =
        absolute_path_from_tool_arg(root_arg, "root").map_err(ToolFailure::invalid_input)?;
    if !search_root.exists() {
        return Err(ToolFailure::new(format!(
            "Root path does not exist: {}",
            search_root.display()
        )));
    }
    if !search_root.is_dir() {
        return Err(ToolFailure::new(format!(
            "Root path is not a directory: {}",
            search_root.display()
        )));
    }
    let search_root = search_root
        .canonicalize()
        .map_err(|error| ToolFailure::new(format!("Failed to resolve root path: {}", error)))?;

    let options = GlobSearchOptions {
        pattern,
        case_sensitive: input
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        max_results,
        walk: WalkOptions {
            include_hidden: input
                .get("include_hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            include_ignored: input
                .get("include_ignored")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            follow_symlinks: input
                .get("follow_symlinks")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
    };

    let result = match find_files(&search_root, &search_root, &options) {
        Ok(result) => result,
        Err(error) => return Err(ToolFailure::new(error)),
    };

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "query": {
            "root": search_root.display().to_string(),
            "pattern": options.pattern,
            "case_sensitive": options.case_sensitive,
            "include_hidden": options.walk.include_hidden,
            "include_ignored": options.walk.include_ignored,
            "follow_symlinks": options.walk.follow_symlinks,
            "max_results": options.max_results,
        },
        "stats": result.stats,
        "files": result.files,
    })))
}
