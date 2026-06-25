use crate::llm::services::file_changes::{resolve_tool_path, write_file_simple};
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
    _app: &AppHandle,
    _conversation_id: Option<&str>,
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

    let target = resolve_path(file_path).map_err(|e| ToolFailure::invalid_input(e))?;
    let existed = target.exists();

    let path = write_file_simple(&target, content).map_err(ToolFailure::new)?;

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "file_path": file_path,
        "created": !existed,
        "path": path
    })))
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