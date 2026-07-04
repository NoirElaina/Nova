use crate::llm::tools::shared::read_state;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::file_io::{read_file_meta, resolve_tool_path, write_text_content_lf, FileEncoding};
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
- `content` is the text content to write. The model sent explicit line endings in `content` and meant them — they are written as-is, not rewritten to match the old file's line endings.
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

    let target = resolve_path(file_path).map_err(ToolFailure::invalid_input)?;
    let existed = target.exists();

    // 行尾策略：模型在 content 里写的就是它要的行尾，写什么落什么，不还原旧文件行尾。
    // 保留旧 CRLF 会在覆盖 CRLF 文件时把 bash 脚本写入 \r，损坏脚本。
    // 仅按原文件 encoding 还原字节编码（UTF-8 / UTF-8 BOM / UTF-16 LE/BE）。
    let encoding = if existed {
        // 覆盖已有文件：要求先读过且未被外部改动，保留原编码。
        // 行尾不还原，只用 encoding。
        let (original, meta) = read_file_meta(&target)
            .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?;
        // TOCTOU 缓解：read→check→write 串成 sync 调用，中间不 await。
        // ensure_editable 用 mtime+content 二级检测，通过后立即写盘。
        read_state::ensure_editable(conversation_id, &target, &original)
            .map_err(ToolFailure::new)?;
        meta.encoding
    } else {
        // 新建文件：UTF-8 / 无 BOM。
        FileEncoding::Utf8
    };

    // 落盘走 atomic_write（tempfile + rename + 权限保留 + symlink 解析）。
    // 不还原行尾，模型 content 直接落盘。
    let path = write_text_content_lf(&target, content, encoding).map_err(ToolFailure::new)?;

    // 刷新读取状态，使后续 Edit/Write 可继续。
    // 注意：record 内部会重新读 mtime，确保拿到写入后的最新 mtime。
    read_state::record(conversation_id, &target, content);

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
