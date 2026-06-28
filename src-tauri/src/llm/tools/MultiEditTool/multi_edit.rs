use crate::llm::tools::shared::edit_replacers::apply_replace;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::file_io::{read_file_meta, resolve_tool_path, write_file_with_meta};
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission))
}

pub fn tool() -> Tool {
    Tool {
        name: "MultiEdit".into(),
        description: r#"Performs multiple exact string replacements on the same file in a single call.

Use this tool when you need to make 2+ distinct edits to the same file. It is more efficient than calling Edit multiple times because:
- Only one tool call round-trip (lower latency, fewer tokens)
- The file is read once and written once (atomic batch)

Constraints:
- Each edit's `old_string` must be unique in the file (or use `replace_all: true` for that edit).
- Edits are applied sequentially in array order. Each subsequent edit sees the result of the previous one, so if edit #1 changes a line, edit #2's `old_string` should match the NEW content (not the original).
- Same fuzzy matching as Edit: line-trimmed, block-anchor, whitespace-normalized, indentation-flexible, escape-normalized are tried in order when exact match fails.
- `file_path` must be an absolute path.
- If any edit fails, the entire batch is aborted and NO changes are written (atomic).

Each edit object: { old_string: string, new_string: string, replace_all?: boolean }
"#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "edits": {
                    "type": "array",
                    "description": "Ordered list of edits to apply. Each edit sees the result of the previous one.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": {
                                "type": "string",
                                "description": "The text to replace"
                            },
                            "new_string": {
                                "type": "string",
                                "description": "The text to replace it with"
                            },
                            "replace_all": {
                                "type": "boolean",
                                "description": "Replace all occurrences of old_string (default false)"
                            }
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["file_path", "edits"]
        }),
    }
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    crate::llm::utils::permissions::describe_file_write_permission(
        "MultiEdit",
        "批量编辑文件",
        "file_path",
        input,
    )
}

struct ParsedEdit {
    old_string: String,
    new_string: String,
    replace_all: bool,
}

fn parse_edit(value: &Value, idx: usize) -> Result<ParsedEdit, ToolFailure> {
    let old_string = value
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ToolFailure::invalid_input(format!("edits[{}].old_string is required (string)", idx))
        })?;
    let new_string = value
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ToolFailure::invalid_input(format!("edits[{}].new_string is required (string)", idx))
        })?;
    let replace_all = value
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if old_string == new_string {
        return Err(ToolFailure::invalid_input(format!(
            "edits[{}]: old_string and new_string must be different",
            idx
        )));
    }
    if old_string.is_empty() {
        return Err(ToolFailure::invalid_input(format!(
            "edits[{}]: old_string must not be empty",
            idx
        )));
    }

    Ok(ParsedEdit {
        old_string: old_string.to_string(),
        new_string: new_string.to_string(),
        replace_all,
    })
}

async fn execute_async(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let file_path = input
        .get("file_path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: file_path"))?;

    let edits_raw = input
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: edits (array)"))?;

    if edits_raw.is_empty() {
        return Err(ToolFailure::invalid_input("edits must not be empty"));
    }

    let target = resolve_tool_path(file_path).map_err(ToolFailure::invalid_input)?;

    if !target.exists() {
        return Err(ToolFailure::new(format!(
            "File does not exist: {}. Use Write to create a new file.",
            file_path
        )));
    }

    // 解析所有 edit
    let mut edits: Vec<ParsedEdit> = Vec::with_capacity(edits_raw.len());
    for (idx, item) in edits_raw.iter().enumerate() {
        edits.push(parse_edit(item, idx)?);
    }

    // 读取文件一次（解码 + 剥 BOM + CRLF→LF），记录原始编码与行尾。
    let (mut content, meta) = read_file_meta(&target)
        .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?;

    // 顺序应用所有 edit。任一失败则整批回滚（不写入）。
    let mut applied_count = 0usize;
    for (idx, edit) in edits.iter().enumerate() {
        let (new_content, replaced) =
            apply_replace(&content, &edit.old_string, &edit.new_string, edit.replace_all)
                .map_err(|e| {
                    ToolFailure::new(format!(
                        "edits[{}] failed (no changes written, file unchanged): {}",
                        idx, e
                    ))
                })?;
        content = new_content;
        applied_count += replaced;
    }

    // 全部成功后写回——按原始编码与行尾还原。
    write_file_with_meta(&target, &content, &meta).map_err(ToolFailure::new)?;

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "file_path": file_path,
        "edits_applied": edits.len(),
        "occurrences_replaced": applied_count
    })))
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
