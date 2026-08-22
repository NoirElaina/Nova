use crate::llm::tools::shared::edit_replacers::apply_replace;
use crate::llm::tools::shared::read_state;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::file_io::{read_file_meta, resolve_tool_path, write_file_with_meta};
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, Some(permission), ToolDisclosure::Core)
}

pub fn tool() -> Tool {
    Tool {
        name: "Edit".into(),
        description: r#"Performs exact string replacement in an existing file. This is the precision tool for surgical changes — use Write for new files or full rewrites, and MultiEdit for several edits to the same file in one atomic call.

## Before you edit
- Base `old_string` on the file's actual current content, ideally from a recent Read — editing from memory risks stale, mismatched text.
- Copy text character-for-character, including all indentation, spaces, and newlines.

## Line-number prefix (critical)
Read tool output prefixes each line with: spaces + line number + tab (e.g. `     1\t`). Everything AFTER the tab is the real file content. NEVER include any part of the line number prefix in `old_string` or `new_string`.

## Matching rules
- With `replace_all: false` (default), `old_string` must be unique in the file — if it appears multiple times, include surrounding context lines to disambiguate, or set `replace_all: true` to replace every occurrence.
- `new_string` must differ from `old_string`; an empty `old_string` is rejected.
- The matcher is fault-tolerant: after exact matching fails, it tries line-trimmed, block-anchor (Levenshtein similarity), whitespace-normalized, indentation-flexible, and escape-normalized matching in that order. Prefer exact matches — fuzzy matching is a safety net, not a shortcut.
- `file_path` must be an absolute path to an existing file (creating files is Write's job).

## Failure recovery
- If an edit fails, the error includes the CLOSEST block found in the file (with its line number) when one exists — compare it against your `old_string`, adopt the exact file content, and retry. Only Re-Read if no closest block is shown or it is clearly unrelated.
- After three consecutive failures on the same file, stop patching: rewrite the whole file (or the containing function) with Write instead of a fourth attempt.

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

    // 归一化 new_string 为 LF，避免模型输出的 \r\n 在 CRLF 文件还原时产生 \r\r\n 损坏。
    // 先把 \r\n 归一成 \n，再按原始行尾还原，避免 \r\r\n。
    let new_string_lf = new_string.replace("\r\n", "\n");

    // 使用 fuzzy matcher 链：精确匹配 → 行 trim → 锚点 → 空白归一化 → ...
    // 这避免了 AI 因一两个空格差异就失败重试。
    let (modified, replaced_count) =
        apply_replace(&original, old_string, &new_string_lf, replace_all)
            .map_err(ToolFailure::new)?;

    // 写回时按原始编码与行尾还原——CRLF 文件保持 CRLF，带 BOM 的文件保持 BOM。
    // 落盘走 atomic_write（tempfile + rename + 权限保留 + symlink 解析）。
    write_file_with_meta(&target, &modified, &meta).map_err(ToolFailure::new)?;

    // 刷新读取状态，使同一轮内的后续编辑可继续。
    read_state::record(conversation_id, &target, &modified);

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
