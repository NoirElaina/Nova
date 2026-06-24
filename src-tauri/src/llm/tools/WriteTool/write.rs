use crate::llm::services::file_changes::{commit_drafts, read_file_utf8, resolve_tool_path, FileChangeDraft};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission))
}

pub fn tool() -> Tool {
    Tool {
        name: "Write".into(),
        description: r#"Write a file to the local filesystem. Creates the file if it does not exist, overwrites it if it does. Creates parent directories as needed.

- `file_path` must be an absolute path.
- `content` is the text content to write.
- This tool will overwrite the existing file if there is one."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        }),
    }
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let _path = input
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    crate::llm::utils::permissions::describe_file_write_permission(
        "Write",
        "写入文件",
        "file_path",
        input,
    )
}

fn resolve_path(raw: &str) -> Result<PathBuf, String> {
    resolve_tool_path(raw)
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

    let content = input
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: content"))?;

    let target = resolve_path(file_path)
        .map_err(|e| ToolFailure::invalid_input(e))?;

    let before = if target.exists() {
        Some(read_file_utf8(&target)
            .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?)
    } else {
        None
    };

    let draft = FileChangeDraft {
        before,
        path: target,
        after: Some(content.to_string()),
    };

    match commit_drafts(app, conversation_id, "Write", vec![draft]).await {
        Ok(result) => Ok(ToolOutcome::json(json!({
            "ok": true,
            "file_path": file_path,
            "changed_files": result.files.len(),
            "change_batch_id": result.change_batch_id
        }))),
        Err(error) => Err(ToolFailure::new(error)),
    }
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
