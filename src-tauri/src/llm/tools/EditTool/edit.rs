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
        name: "Edit".into(),
        description: r#"Performs precise string replacement in an existing file. This is the precision tool for surgical changes — use Write for new files or full rewrites, and MultiEdit for several edits to the same file in one atomic call.

## Before you edit
- Base `old_string` on the file's actual current content from a Read call.
- Copy text character-for-character, including all indentation, spaces, and newlines.

## Line-number prefix (critical)
Read tool output prefixes each line with: spaces + line number + tab (e.g. `     1\t`). Everything AFTER the tab is the real file content. NEVER include any part of the line number prefix in `old_string` or `new_string`.

## Matching rules
- With `replace_all: false` (default), `old_string` must be unique in the file — if it appears multiple times, include surrounding context lines to disambiguate, or set `replace_all: true` to replace every occurrence.
- `new_string` must differ from `old_string`; an empty `old_string` is rejected.
- The matcher first attempts an exact match. If that fails, it falls back to a line-trimmed match (ignoring trailing whitespace differences).
- `file_path` must be an absolute path to an existing file (creating files is Write's job).

## Failure recovery
- If an edit fails, the error response provides the CLOSEST matching block found in the file with line numbers — compare it against your `old_string`, copy the exact file content, and retry.
- After three consecutive failures on the same file, stop patching: rewrite the file with Write instead.

## Common mistakes
- Including the Read output's line-number prefix in `old_string`.
- Choosing an `old_string` that occurs multiple times (ambiguous match).
- Re-indenting or trimming code when copying it into `old_string`.
- Using Edit on a file that does not exist yet."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with (must be different from old_string)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences of old_string (default false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        }),
    }
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    crate::llm::utils::permissions::describe_file_write_permission(
        "Edit",
        "编辑文件",
        "file_path",
        input,
    )
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

    let old_string = input
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: old_string"))?;

    let new_string = input
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: new_string"))?;

    let replace_all = input
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let res = crate::agent::tools::edit::execute_exact_edit(file_path, old_string, new_string, replace_all)
        .await
        .map_err(ToolFailure::new)?;

    Ok(ToolOutcome::json(res))
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        execute_async(&app, conversation_id.as_deref(), input).await
    })
}
