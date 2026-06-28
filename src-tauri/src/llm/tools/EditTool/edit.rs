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
        name: "Edit".into(),
        description: r#"Performs exact string replacement in an existing file.

- The `old_string` should match the file content exactly, but the matcher is fault-tolerant: it will also try line-trimmed, block-anchor (with Levenshtein similarity), whitespace-normalized, indentation-flexible, and escape-normalized matching in that order before giving up.
- Use `replace_all: true` to replace every occurrence of `old_string`; when false (default), the string must appear exactly once in the file (or be uniquely identifiable via the fuzzy matchers).
- `new_string` must differ from `old_string`.
- `file_path` must be an absolute path.

This is the precision editing tool — use it for surgical changes. For full-file writes or new files, use Write. For multiple edits to the same file in one call, use MultiEdit."#
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

    if old_string == new_string {
        return Err(ToolFailure::invalid_input(
            "old_string and new_string must be different",
        ));
    }

    if old_string.is_empty() {
        return Err(ToolFailure::invalid_input("old_string must not be empty"));
    }

    let target = resolve_tool_path(file_path).map_err(ToolFailure::invalid_input)?;

    if !target.exists() {
        return Err(ToolFailure::new(format!(
            "File does not exist: {}. Use Write to create a new file.",
            file_path
        )));
    }

    // read_file_meta 解码为 UTF-8、剥离 BOM、CRLF→LF，并返回原始编码与行尾元信息。
    // original 是模型应看到的归一化内容（纯 LF、无 BOM）；meta 用于写回时还原。
    let (original, meta) = read_file_meta(&target)
        .map_err(|e| ToolFailure::new(format!("Error reading {}: {}", file_path, e)))?;

    // 使用 fuzzy matcher 链：精确匹配 → 行 trim → 锚点 → 空白归一化 → ...
    // 这避免了 AI 因一两个空格差异就失败重试。
    let (modified, replaced_count) = apply_replace(&original, old_string, new_string, replace_all)
        .map_err(ToolFailure::new)?;

    // 写回时按原始编码与行尾还原——CRLF 文件保持 CRLF，带 BOM 的文件保持 BOM。
    write_file_with_meta(&target, &modified, &meta).map_err(ToolFailure::new)?;

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "file_path": file_path,
        "occurrences_replaced": replaced_count
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
