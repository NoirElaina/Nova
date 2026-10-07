use crate::agent::tools::edit::{execute_exact_multi_edit, EditOperation};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission), ToolDisclosure::Core)
}

pub fn tool() -> Tool {
    Tool {
        name: "MultiEdit".into(),
        description: r#"Performs multiple precise string replacements on the same file in a single atomic call. The default choice whenever a change touches 2+ places in one file — one round trip instead of several Edit calls, and the file is read and written once.

## When to choose which editing tool
- One change in one file → Edit.
- Multiple changes in one file → MultiEdit (this tool).
- New file, or rewriting most of an existing file → Write.

## How to use it
- Pass an ordered list of edits; each edit is `{ old_string, new_string, replace_all? }`.
- Edits apply sequentially in array order — each edit sees the result of the previous one. If edit #1 changes a line, edit #2's `old_string` must match the NEW content, not the original.
- Each `old_string` must be unique in the current file state (or set `replace_all: true` for that edit). When ambiguous, include surrounding context lines to disambiguate.
- Base `old_string` values on the file's actual current content from a recent Read.
- The matcher attempts exact match first, falling back to line-trimmed matching (ignoring trailing whitespace differences).
- `file_path` must be an absolute path to an existing file.

## Line-number prefix (critical)
Read tool output prefixes each line with: spaces + line number + tab (e.g. `     1\t`). Everything AFTER the tab is the real file content. NEVER include any part of the line number prefix in `old_string` or `new_string`.

## Atomicity and failure recovery
- If any edit fails, the entire batch is aborted and NO changes are written — a failed MultiEdit leaves the file untouched.
- On failure, the error response identifies which edit failed and displays the CLOSEST matching block in the file with line numbers to help you correct the batch.
- After three consecutive failures on the same file, stop patching: rewrite the whole file with Write instead.

## Common mistakes
- Writing edit #2's `old_string` against the ORIGINAL file content when edit #1 already changed that region.
- Including the Read output's line-number prefix in `old_string`.
- Choosing `old_string` values that overlap each other or occur multiple times.
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

fn parse_edit(value: &Value, idx: usize) -> Result<EditOperation, ToolFailure> {
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

    Ok(EditOperation {
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

    let mut edits: Vec<EditOperation> = Vec::with_capacity(edits_raw.len());
    for (idx, item) in edits_raw.iter().enumerate() {
        edits.push(parse_edit(item, idx)?);
    }

    let res = execute_exact_multi_edit(file_path, &edits)
        .await
        .map_err(ToolFailure::new)?;

    Ok(ToolOutcome::json(res))
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
