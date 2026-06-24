use crate::llm::services::file_changes::{commit_drafts, read_file_utf8, resolve_tool_path, FileChangeDraft};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission))
}

pub fn tool() -> Tool {
    Tool {
        name: "Edit".into(),
        description: r#"Performs exact string replacement in an existing file.

- The `old_string` must match the file EXACTLY, including all whitespace and indentation. The edit fails otherwise.
- Use `replace_all: true` to replace every occurrence of `old_string`; when false (default), the string must appear exactly once in the file.
- `new_string` must differ from `old_string`.
- `file_path` must be an absolute path.

This is the precision editing tool — use it for surgical changes. For full-file writes or new files, use Write."#
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
    app: &AppHandle,
    conversation_id: Option<&str>,
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

    if old_string == new_string {
        return Err(ToolFailure::invalid_input(
            "old_string and new_string must be different",
        ));
    }

    if old_string.is_empty() {
        return Err(ToolFailure::invalid_input("old_string must not be empty"));
    }

    let target = resolve_tool_path(file_path)
        .map_err(|e| ToolFailure::invalid_input(e))?;

    if !target.exists() {
        return Err(ToolFailure::new(format!(
            "File does not exist: {}. Use Write to create a new file.",
            file_path
        )));
    }

    let original = read_file_utf8(&target)
        .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?;

    let occurrences = count_matches(&original, old_string);

    if occurrences == 0 {
        return Err(ToolFailure::new(format!(
            "old_string not found in file: {}\n\nTip: Read the file first to get exact content and indentation.",
            file_path
        )));
    }

    if !replace_all && occurrences > 1 {
        return Err(ToolFailure::new(format!(
            "old_string found {} times in file (not unique). Use replace_all: true to replace all occurrences, or provide more context to make old_string unique.",
            occurrences
        )));
    }

    let modified = if replace_all {
        original.replace(old_string, new_string)
    } else {
        original.replacen(old_string, new_string, 1)
    };

    let draft = FileChangeDraft {
        before: Some(original),
        path: target,
        after: Some(modified),
    };

    match commit_drafts(app, conversation_id, "Edit", vec![draft]).await {
        Ok(result) => {
            let replaced_count = if replace_all {
                occurrences
            } else {
                1
            };
            Ok(ToolOutcome::json(json!({
                "ok": true,
                "file_path": file_path,
                "occurrences_replaced": replaced_count,
                "changed_files": result.files.len(),
                "change_batch_id": result.change_batch_id
            })))
        }
        Err(error) => Err(ToolFailure::new(error)),
    }
}

fn count_matches(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count()
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
