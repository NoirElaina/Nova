use crate::llm::tools::shared::read_state;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::file_io::{
    read_file_meta, resolve_tool_path, write_file_with_meta, FileEncoding, FileMeta, LineEnding,
};
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

    let target = resolve_path(file_path).map_err(|e| ToolFailure::invalid_input(e))?;
    let existed = target.exists();

    // 归一化模型内容为 LF；write_file_with_meta 按目标 meta 还原行尾。
    let content_lf = content.replace("\r\n", "\n");

    let meta = if existed {
        // 覆盖已有文件：要求先读过且未被外部改动，并保留原编码与行尾。
        let (original, meta) = read_file_meta(&target)
            .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?;
        read_state::ensure_editable(conversation_id, &target, &original)
            .map_err(ToolFailure::new)?;
        meta
    } else {
        // 新建文件：UTF-8 / LF。
        FileMeta {
            encoding: FileEncoding::Utf8,
            line_ending: LineEnding::Lf,
        }
    };

    let path = write_file_with_meta(&target, &content_lf, &meta).map_err(ToolFailure::new)?;

    // 刷新读取状态，使后续 Edit/Write 可继续。
    read_state::record(conversation_id, &target, &content_lf);

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