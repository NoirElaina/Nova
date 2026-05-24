use crate::llm::services::search::{grep_text, TextSearchMode, TextSearchOptions, WalkOptions};
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

const MAX_OUTPUT_CHARS: usize = 10_000;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_sync_stub, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "grep_search".into(),
        description: "Search text in files inside the conversation WorkspaceRoot. Uses ripgrep-style ignore handling and grep semantics. Returns structured JSON with stats, matches, and optional context lines.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pattern": { "type": "string", "minLength": 1, "description": "The literal text or regex pattern to search for." },
                "path": { "type": "string", "description": "Workspace-relative directory or file to search. Defaults to WorkspaceRoot." },
                "mode": { "type": "string", "enum": ["literal", "regex"], "description": "Search mode. Defaults to literal." },
                "case_sensitive": { "type": "boolean", "description": "Use case-sensitive matching. Defaults to true." },
                "include_globs": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 },
                    "description": "Optional file glob filters, for example [\"**/*.rs\", \"**/*.ts\"]. Bare filename globs match at any depth."
                },
                "context_lines": { "type": "integer", "minimum": 0, "maximum": 5, "description": "Number of lines to include before and after each match. Defaults to 0." },
                "include_hidden": { "type": "boolean", "description": "Include hidden files and directories. Defaults to false." },
                "include_ignored": { "type": "boolean", "description": "Include files ignored by .ignore, .gitignore, .git/info/exclude, and global gitignore. Defaults to false." },
                "follow_symlinks": { "type": "boolean", "description": "Follow symbolic links while walking directories. Defaults to false." },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Maximum number of matching lines. Defaults to 200." }
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

fn parse_mode(input: &Value) -> Result<TextSearchMode, String> {
    match input
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("literal")
    {
        "literal" => Ok(TextSearchMode::Literal),
        "regex" => Ok(TextSearchMode::Regex),
        other => Err(format!("Invalid mode '{}'. Use 'literal' or 'regex'.", other)),
    }
}

fn parse_include_globs(input: &Value) -> Result<Vec<String>, String> {
    let Some(value) = input.get("include_globs") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err("include_globs must be an array of strings".to_string());
    };

    let mut globs = Vec::new();
    for item in items {
        let Some(glob) = item.as_str().map(str::trim).filter(|value| !value.is_empty()) else {
            return Err("include_globs must contain only non-empty strings".to_string());
        };
        globs.push(glob.to_string());
    }
    Ok(globs)
}

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    let pattern = match input.get("pattern").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => return json!({ "ok": false, "error": "Missing 'pattern' argument" }).to_string(),
    };
    let path_arg = input
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let mode = match parse_mode(&input) {
        Ok(mode) => mode,
        Err(error) => return json!({ "ok": false, "error": error }).to_string(),
    };
    let include_globs = match parse_include_globs(&input) {
        Ok(globs) => globs,
        Err(error) => return json!({ "ok": false, "error": error }).to_string(),
    };
    let max_results = input
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(200)
        .clamp(1, 2000);
    let context_lines = input
        .get("context_lines")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0)
        .clamp(0, 5);

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

    let options = TextSearchOptions {
        pattern,
        mode,
        case_sensitive: input
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        include_globs,
        context_lines,
        max_results,
        max_output_chars: MAX_OUTPUT_CHARS,
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

    let result = match grep_text(&workspace_root, &target, &options) {
        Ok(result) => result,
        Err(error) => return json!({ "ok": false, "error": error }).to_string(),
    };

    json!({
        "ok": true,
        "query": {
            "path": path_arg,
            "pattern": options.pattern,
            "mode": options.mode,
            "case_sensitive": options.case_sensitive,
            "include_globs": options.include_globs,
            "context_lines": options.context_lines,
            "include_hidden": options.walk.include_hidden,
            "include_ignored": options.walk.include_ignored,
            "follow_symlinks": options.walk.follow_symlinks,
            "max_results": options.max_results,
        },
        "stats": result.stats,
        "matches": result.matches,
    })
    .to_string()
}
