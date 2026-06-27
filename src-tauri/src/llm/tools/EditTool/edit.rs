use crate::llm::tools::shared::read_state::global_registry;
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::file_io::{read_file_utf8, resolve_tool_path, write_file_preserving};
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
- You MUST read the file with the Read tool before editing it. Edits will be rejected if the file hasn't been read in the current session.
- If the file was modified since you last read it (by you, the user, or a linter), the edit will be rejected — read it again first.
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

    // 先读后写检查：当前会话必须已读过此文件，且读取后文件未被外部修改。
    // 这避免 AI 凭可能过时的记忆盲改文件。
    if let Err(reason) = global_registry().check_editable(conversation_id, &target) {
        return Err(ToolFailure::new(reason.message()));
    }

    // read_file_utf8 同时 strip BOM 并返回 had_bom 标记。
    // original 是 AI 看到的内容（无 BOM），had_bom 用于写回时恢复 BOM。
    let (original, had_bom) = read_file_utf8(&target)
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

    // 写回时保留 BOM 状态——有 BOM 的 Windows 文件编辑后仍然有 BOM。
    write_file_preserving(&target, &modified, had_bom).map_err(ToolFailure::new)?;

    // 登记新内容到 read_state，让后续连续编辑无需重新 Read。
    global_registry().record_edit(conversation_id, &target, modified);

    let replaced_count = if replace_all { occurrences } else { 1 };
    Ok(ToolOutcome::json(json!({
        "ok": true,
        "file_path": file_path,
        "occurrences_replaced": replaced_count
    })))
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
