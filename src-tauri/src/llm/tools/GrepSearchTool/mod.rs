use crate::llm::services::search::{
    grep_text, TextSearchOptions, TextSearchOutputMode, WalkOptions,
};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

const DEFAULT_HEAD_LIMIT: usize = 250;
const MAX_HEAD_LIMIT: usize = 10_000;
const MAX_CONTEXT_LINES: usize = 20;
const MAX_OUTPUT_CHARS: usize = 20_000;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, Some(permission))
}

pub fn tool() -> Tool {
    Tool {
        name: "grep_search".into(),
        description: "Search file contents with ripgrep-style regex semantics. Supports content, files_with_matches, and count modes; glob/type filters; context lines; pagination with head_limit and offset; mtime sorting for file results.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The regular expression pattern to search for in file contents."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search. Absolute paths are allowed; relative paths resolve under the conversation WorkspaceRoot. Defaults to WorkspaceRoot."
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern filter, for example \"*.rs\" or \"*.{ts,tsx}\". Whitespace separates patterns; commas also split patterns outside brace groups."
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "content shows matching lines; files_with_matches shows files; count shows per-file counts. Defaults to files_with_matches."
                },
                "-B": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_CONTEXT_LINES,
                    "description": "Number of lines before each match. Only applies to content mode."
                },
                "-A": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_CONTEXT_LINES,
                    "description": "Number of lines after each match. Only applies to content mode."
                },
                "-C": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_CONTEXT_LINES,
                    "description": "Number of lines before and after each match. Alias for context."
                },
                "context": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_CONTEXT_LINES,
                    "description": "Number of lines before and after each match. Takes precedence over -A and -B."
                },
                "-n": {
                    "type": "boolean",
                    "description": "Show line numbers in content mode. Defaults to true."
                },
                "-i": {
                    "type": "boolean",
                    "description": "Use case-insensitive search."
                },
                "type": {
                    "type": "string",
                    "description": "File type filter. Common values: rust, js, ts, tsx, py, go, java, json, md, vue, shell, powershell."
                },
                "head_limit": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_HEAD_LIMIT,
                    "description": "Limit output to first N lines or entries after offset. Defaults to 250. Pass 0 for unlimited."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Skip first N lines or entries before applying head_limit. Defaults to 0."
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multiline regex mode where matches may span lines and . matches newlines. Defaults to false."
                }
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

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let raw_path = input
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let normalized = normalize_path_for_permission(raw_path);
    let warning = read_permission_warning(raw_path, &normalized);

    Some(ToolPermissionDescriptor {
        signature: format!("grep_search:{}", normalized),
        preview: format!("搜索文件内容（grep_search）：{}", truncate_chars(raw_path, 220)),
        warning: warning.clone(),
        needs_approval: warning.is_some(),
    })
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let pattern = input
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolFailure::invalid_input("Missing 'pattern' argument"))?
        .to_string();

    let output_mode = parse_output_mode(&input)?;
    let glob_patterns = parse_glob_patterns(&input)?;
    let type_name = input
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let type_globs = match type_name.as_deref() {
        Some(value) => type_globs(value).map_err(ToolFailure::invalid_input)?,
        None => Vec::new(),
    };

    let (before_context, after_context) = parse_context(&input);
    let head_limit = parse_head_limit(&input);
    let offset = input
        .get("offset")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0);

    let workspace_root =
        crate::command::workspace::workspace_root_for_conversation(app, conversation_id)
            .map_err(ToolFailure::new)?;
    let path_arg = input
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let target = resolve_search_path(&workspace_root, path_arg).map_err(ToolFailure::new)?;
    if !target.exists() {
        return Err(ToolFailure::new(format!(
            "Search path does not exist: {}",
            target.display()
        )));
    }
    if !target.is_file() && !target.is_dir() {
        return Err(ToolFailure::new(format!(
            "Search path is not a file or directory: {}",
            target.display()
        )));
    }

    let options = TextSearchOptions {
        pattern,
        case_sensitive: !input.get("-i").and_then(Value::as_bool).unwrap_or(false),
        glob_patterns,
        type_globs,
        exclude_globs: default_exclude_globs(),
        output_mode,
        before_context,
        after_context,
        show_line_numbers: input.get("-n").and_then(Value::as_bool).unwrap_or(true),
        head_limit,
        offset,
        multiline: input
            .get("multiline")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        max_output_chars: MAX_OUTPUT_CHARS,
        walk: WalkOptions {
            include_hidden: true,
            include_ignored: false,
            follow_symlinks: false,
        },
    };

    let result = grep_text(&workspace_root, &target, &options).map_err(ToolFailure::new)?;
    Ok(ToolOutcome::json(json!({
        "ok": true,
        "query": {
            "path": path_arg,
            "pattern": options.pattern,
            "glob": options.glob_patterns,
            "type": type_name,
            "output_mode": options.output_mode,
            "context_before": options.before_context,
            "context_after": options.after_context,
            "line_numbers": options.show_line_numbers,
            "case_insensitive": !options.case_sensitive,
            "head_limit": options.head_limit,
            "offset": options.offset,
            "multiline": options.multiline,
        },
        "result": result,
    })))
}

fn parse_output_mode(input: &Value) -> Result<TextSearchOutputMode, ToolFailure> {
    match input
        .get("output_mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("files_with_matches")
    {
        "content" => Ok(TextSearchOutputMode::Content),
        "files_with_matches" => Ok(TextSearchOutputMode::FilesWithMatches),
        "count" => Ok(TextSearchOutputMode::Count),
        other => Err(ToolFailure::invalid_input(format!(
            "Invalid output_mode '{}'. Use content, files_with_matches, or count.",
            other
        ))),
    }
}

fn parse_glob_patterns(input: &Value) -> Result<Vec<String>, ToolFailure> {
    let Some(raw) = input
        .get("glob")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(Vec::new());
    };

    let mut patterns = Vec::new();
    for token in raw.split_whitespace() {
        if token.contains('{') && token.contains('}') {
            patterns.push(token.to_string());
            continue;
        }

        for part in token.split(',') {
            let part = part.trim();
            if !part.is_empty() {
                patterns.push(part.to_string());
            }
        }
    }

    if patterns.is_empty() {
        return Err(ToolFailure::invalid_input("glob cannot be empty"));
    }
    Ok(patterns)
}

fn parse_context(input: &Value) -> (usize, usize) {
    if let Some(value) = input.get("context").and_then(Value::as_u64) {
        let value = (value as usize).min(MAX_CONTEXT_LINES);
        return (value, value);
    }
    if let Some(value) = input.get("-C").and_then(Value::as_u64) {
        let value = (value as usize).min(MAX_CONTEXT_LINES);
        return (value, value);
    }

    let before = input
        .get("-B")
        .and_then(Value::as_u64)
        .map(|value| (value as usize).min(MAX_CONTEXT_LINES))
        .unwrap_or(0);
    let after = input
        .get("-A")
        .and_then(Value::as_u64)
        .map(|value| (value as usize).min(MAX_CONTEXT_LINES))
        .unwrap_or(0);
    (before, after)
}

fn parse_head_limit(input: &Value) -> Option<usize> {
    match input.get("head_limit").and_then(Value::as_u64) {
        Some(0) => None,
        Some(value) => Some((value as usize).min(MAX_HEAD_LIMIT)),
        None => Some(DEFAULT_HEAD_LIMIT),
    }
}

fn resolve_search_path(workspace_root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = trim_wrapping_quotes(raw_path.trim());
    let expanded = expand_home(trimmed);
    let path = PathBuf::from(expanded.as_ref());
    if path.is_absolute() {
        return Ok(path);
    }
    crate::llm::services::file_changes::resolve_tool_path(workspace_root, expanded.as_ref())
}

fn type_globs(name: &str) -> Result<Vec<String>, String> {
    let patterns = match name.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => &["*.rs"][..],
        "js" | "javascript" => &["*.js", "*.mjs", "*.cjs", "*.jsx"],
        "jsx" => &["*.jsx"],
        "ts" | "typescript" => &["*.ts", "*.mts", "*.cts", "*.tsx"],
        "tsx" => &["*.tsx"],
        "py" | "python" => &["*.py", "*.pyw"],
        "go" => &["*.go"],
        "java" => &["*.java"],
        "kt" | "kotlin" => &["*.kt", "*.kts"],
        "c" => &["*.c", "*.h"],
        "cpp" | "c++" => &["*.cc", "*.cpp", "*.cxx", "*.hpp", "*.hh", "*.hxx"],
        "cs" | "csharp" => &["*.cs"],
        "php" => &["*.php"],
        "rb" | "ruby" => &["*.rb"],
        "swift" => &["*.swift"],
        "scala" => &["*.scala", "*.sc"],
        "html" => &["*.html", "*.htm"],
        "css" => &["*.css"],
        "scss" => &["*.scss"],
        "sass" => &["*.sass"],
        "json" => &["*.json", "*.jsonc"],
        "yaml" | "yml" => &["*.yaml", "*.yml"],
        "toml" => &["*.toml"],
        "md" | "markdown" => &["*.md", "*.markdown", "*.mdx"],
        "sql" => &["*.sql"],
        "sh" | "shell" | "bash" => &["*.sh", "*.bash", "*.zsh", "*.fish"],
        "powershell" | "ps1" => &["*.ps1", "*.psm1", "*.psd1"],
        "vue" => &["*.vue"],
        "svelte" => &["*.svelte"],
        "xml" => &["*.xml"],
        "docker" | "dockerfile" => &["Dockerfile", "*.dockerfile"],
        "make" | "makefile" => &["Makefile", "*.mk"],
        other => {
            return Err(format!(
                "Unsupported type '{}'. Use glob for custom file filters.",
                other
            ))
        }
    };
    Ok(patterns.iter().map(|value| value.to_string()).collect())
}

fn default_exclude_globs() -> Vec<String> {
    [
        ".git/**",
        "**/.git/**",
        ".svn/**",
        "**/.svn/**",
        ".hg/**",
        "**/.hg/**",
        ".bzr/**",
        "**/.bzr/**",
        ".jj/**",
        "**/.jj/**",
        ".sl/**",
        "**/.sl/**",
        ".ssh/**",
        "**/.ssh/**",
        ".aws/**",
        "**/.aws/**",
        ".gnupg/**",
        "**/.gnupg/**",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect()
}

fn expand_home(path: &str) -> Cow<'_, str> {
    if path == "~" {
        if let Some(home) = home_dir_string() {
            return Cow::Owned(home);
        }
    }

    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = home_dir_string() {
            let mut full = PathBuf::from(home);
            full.push(rest);
            return Cow::Owned(full.display().to_string());
        }
    }

    Cow::Borrowed(path)
}

fn home_dir_string() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|value| !value.trim().is_empty()))
}

fn trim_wrapping_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn normalize_path_for_permission(path: &str) -> String {
    path.trim()
        .replace('/', "\\")
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_permission_warning(raw_path: &str, normalized: &str) -> Option<String> {
    if normalized.starts_with("\\\\") || normalized.starts_with("//") {
        return Some(format!(
            "搜索 UNC/网络路径可能触发远程认证或访问外部资源：{}",
            raw_path
        ));
    }

    let sensitive_markers = [
        "\\.ssh\\",
        "\\.aws\\",
        "\\.gnupg\\",
        "\\.config\\git",
        "\\.git\\config",
        "\\id_rsa",
        "\\id_ed25519",
        "\\credentials",
    ];
    if sensitive_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(format!("搜索敏感凭据/配置路径：{}", raw_path));
    }

    None
}

fn truncate_chars(input: &str, limit: usize) -> String {
    let mut chars = input.chars();
    let snippet = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{}...", snippet)
    } else {
        snippet
    }
}
