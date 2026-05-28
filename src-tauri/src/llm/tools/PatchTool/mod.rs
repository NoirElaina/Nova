use crate::llm::services::file_changes::{
    apply_patch_change, multi_edit_change, patch_paths, FileEditResult, MultiEditRequest,
};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tauri::AppHandle;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![
        app_tool(
            apply_patch_tool,
            apply_patch_with_app,
            false,
            Some(apply_patch_permission),
        ),
        app_tool(
            multi_edit_tool,
            multi_edit_with_app,
            false,
            Some(multi_edit_permission),
        ),
    ]
}

fn apply_patch_tool() -> Tool {
    Tool {
        name: "apply_patch".into(),
        description: "Apply a structured multi-file patch to absolute file paths. All writes go through Nova's file-change review service and produce a review record.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Patch text using *** Begin Patch / *** Update File / *** Add File / *** Delete File / @@ hunks / *** End Patch. Every file path in patch headers must be absolute."
                }
            },
            "required": ["patch"]
        }),
    }
}

fn multi_edit_tool() -> Tool {
    Tool {
        name: "multi_edit".into(),
        description: "Apply multiple exact string replacements to absolute file paths. All writes go through Nova's file-change review service and produce a review record.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "path": { "type": "string", "description": "Absolute file path. Relative paths and ~ are rejected." },
                            "old_string": { "type": "string", "description": "Exact string to replace" },
                            "new_string": { "type": "string", "description": "Replacement string" },
                            "expected_replacements": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Exact number of occurrences to replace. Defaults to 1."
                            }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        }),
    }
}

fn apply_patch_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let patch = match input.get("patch").and_then(Value::as_str) {
            Some(patch) => patch,
            None => return Err(ToolFailure::invalid_input("apply_patch requires patch")),
        };
        match apply_patch_change(&app, conversation_id.as_deref(), patch).await {
            Ok(result) => result_json(result),
            Err(error) => Err(ToolFailure::new(error)),
        }
    })
}

fn multi_edit_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let edits = match parse_multi_edits(&input) {
            Ok(edits) => edits,
            Err(error) => return Err(ToolFailure::invalid_input(error)),
        };
        match multi_edit_change(&app, conversation_id.as_deref(), edits).await {
            Ok(result) => result_json(result),
            Err(error) => Err(ToolFailure::new(error)),
        }
    })
}

fn apply_patch_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let patch = input
        .get("patch")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match patch_paths(patch) {
        Ok(paths) => describe_paths_permission("apply_patch", "文件补丁", paths),
        Err(error) => Some(ToolPermissionDescriptor {
            signature: "apply_patch:<invalid>".to_string(),
            preview: "文件补丁（apply_patch）：补丁格式无效".to_string(),
            warning: Some(error),
            needs_approval: false,
        }),
    }
}

fn multi_edit_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let paths = input
        .get("edits")
        .and_then(Value::as_array)
        .map(|edits| {
            edits
                .iter()
                .filter_map(|edit| edit.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    describe_paths_permission("multi_edit", "批量编辑", paths)
}

fn describe_paths_permission(
    tool_name: &str,
    preview_label: &str,
    paths: Vec<String>,
) -> Option<ToolPermissionDescriptor> {
    let unique = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>();

    if unique.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:<empty>", tool_name),
            preview: format!("{}（{}）：未提供文件路径", preview_label, tool_name),
            warning: Some("目标路径为空，无法执行。".to_string()),
            needs_approval: false,
        });
    }

    let paths = unique.iter().cloned().collect::<Vec<_>>();
    let mut warning = None;
    let mut needs_approval = false;
    for path in &paths {
        if let Some(descriptor) = crate::llm::utils::permissions::describe_file_write_permission(
            tool_name,
            preview_label,
            "path",
            &json!({ "path": path }),
        ) {
            if descriptor.needs_approval {
                needs_approval = true;
            }
            if warning.is_none() {
                warning = descriptor.warning;
            }
        }
    }

    let preview_paths = paths.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    let suffix = if paths.len() > 4 {
        format!(" 等 {} 个文件", paths.len())
    } else {
        format!("{} 个文件", paths.len())
    };

    Some(ToolPermissionDescriptor {
        signature: format!(
            "{}:{}",
            tool_name,
            paths
                .iter()
                .map(|path| path.replace('/', "\\").to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("|")
        ),
        preview: format!(
            "{}（{}）：{}{}",
            preview_label, tool_name, preview_paths, suffix
        ),
        warning,
        needs_approval,
    })
}

fn parse_multi_edits(input: &Value) -> Result<Vec<MultiEditRequest>, String> {
    let edits = input
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| "multi_edit requires edits".to_string())?;

    edits
        .iter()
        .enumerate()
        .map(|(index, edit)| {
            let path = required_string(edit, "path")
                .map_err(|error| format!("edits[{}].{}", index, error))?;
            let old_string = required_string(edit, "old_string")
                .map_err(|error| format!("edits[{}].{}", index, error))?;
            let new_string = edit
                .get("new_string")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{}].new_string is required", index))?
                .to_string();
            let expected_replacements = edit
                .get("expected_replacements")
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize;
            Ok(MultiEditRequest {
                path,
                old_string,
                new_string,
                expected_replacements,
            })
        })
        .collect()
}

fn required_string(input: &Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} is required", key))
}

fn result_json(result: FileEditResult) -> Result<ToolOutcome, ToolFailure> {
    let changed_files = result.files.len();
    Ok(ToolOutcome::json(json!({
        "ok": true,
        "files": result.files,
        "changed_files": changed_files,
        "change_batch_id": result.change_batch_id
    })))
}
