use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission))
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    // 写文件属于高风险操作，由工具自己声明“写到哪儿”。
    crate::llm::utils::permissions::describe_file_write_permission(
        "write_file",
        "文件写入",
        "path",
        input,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "write_file".into(),
        description: "Write content to an absolute file path. This completely overwrites the file and records the change for review.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string", "description": "Absolute file path to write. Relative paths and ~ are rejected." },
                "content": { "type": "string", "description": "The content to write" }
            },
            "required": ["path", "content"]
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
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let path = match input.get("path").and_then(|v| v.as_str()) {
        Some(path) if !path.trim().is_empty() => path,
        _ => return Err(ToolFailure::invalid_input("Missing 'path' argument")),
    };
    let content = match input.get("content").and_then(|v| v.as_str()) {
        Some(content) => content,
        _ => return Err(ToolFailure::invalid_input("Missing 'content' argument")),
    };
    match crate::llm::services::file_changes::write_file_change(
        app,
        conversation_id,
        path,
        content,
    )
    .await
    {
        Ok(result) => {
            let changed_files = result.files.len();
            Ok(ToolOutcome::json(json!({
                "ok": true,
                "message": if result.change_batch_id.is_some() { "Successfully wrote to file" } else { "No file changes" },
                "files": result.files,
                "changed_files": changed_files,
                "change_batch_id": result.change_batch_id
            })))
        }
        Err(error) => Err(ToolFailure::new(error)),
    }
}
