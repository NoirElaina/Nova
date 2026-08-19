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
        description: r#"Writes a file to the local filesystem, creating it (with parent directories) if needed, or overwriting it entirely if it exists.

## When to use this tool
- Creating a new file (source code, config, data, scripts).
- Rewriting an existing file whose changes are so extensive that Edit patches would be error-prone — e.g. restructuring most of the file, or after repeated Edit failures.
- NOT for small modifications of existing files — prefer Edit/MultiEdit (minimal diff, reviewable, cheaper). Overwriting throws away the original content.

## Before overwriting an existing file
- Read it first and understand what you are replacing — Write replaces the ENTIRE file; anything not in `content` is lost.
- Preserve the parts that should stay. Reconstruct the full file in `content`, not just the changed region.

## How to use it
- `file_path` must be an absolute path.
- `content` is the complete file text. Line endings are written exactly as sent — they are not rewritten to match the old file's line endings. Use `\n` unless you have a reason otherwise.
- The file's original encoding (UTF-8 / UTF-8 BOM / UTF-16) is preserved on overwrite.

## Common mistakes
- Using Write for a small edit when Edit would do — destroys history clarity and risks losing unrelated content.
- Overwriting a file without reading it first, silently dropping content you did not know about.
- Sending only the changed fragment instead of the complete file content.
- Creating files nobody asked for (docs, backups, scratch copies)."#
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
        // 覆盖已有文件：读取保留原编码。
        // 行尾不还原，只用 encoding。
        let (_original, meta) = read_file_meta(&target)
            .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?;
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
