use crate::llm::services::search::{find_files, GlobSearchOptions, WalkOptions};
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "glob_search".into(),
        description: "Search files by glob pattern inside the conversation WorkspaceRoot. Uses ripgrep-style ignore handling and glob semantics. Returns structured JSON with stats and files.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "root": { "type": "string", "description": "Workspace-relative directory to search. Defaults to WorkspaceRoot." },
                "pattern": { "type": "string", "minLength": 1, "description": "Glob pattern against workspace-relative paths. Supports *, ?, **, character classes, and braces. Bare filename patterns match at any depth." },
                "case_sensitive": { "type": "boolean", "description": "Use case-sensitive glob matching. Defaults to true." },
                "include_hidden": { "type": "boolean", "description": "Include hidden files and directories. Defaults to false." },
                "include_ignored": { "type": "boolean", "description": "Include files ignored by .ignore, .gitignore, .git/info/exclude, and global gitignore. Defaults to false." },
                "follow_symlinks": { "type": "boolean", "description": "Follow symbolic links while walking directories. Defaults to false." },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Maximum number of matches. Defaults to 200." }
            },
            "required": ["pattern"]
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

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    let root_arg = input
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let pattern = match input.get("pattern").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => return json!({ "ok": false, "error": "Missing 'pattern' argument" }).to_string(),
    };
    let max_results = input
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(200)
        .clamp(1, 2000);

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

    let result = match find_files(&workspace_root, &search_root, &options) {
        Ok(result) => result,
        Err(error) => return json!({ "ok": false, "error": error }).to_string(),
    };

    json!({
        "ok": true,
        "query": {
            "root": root_arg,
            "pattern": options.pattern,
            "case_sensitive": options.case_sensitive,
            "include_hidden": options.walk.include_hidden,
            "include_ignored": options.walk.include_ignored,
            "follow_symlinks": options.walk.follow_symlinks,
            "max_results": options.max_results,
        },
        "stats": result.stats,
        "files": result.files,
    })
    .to_string()
}
