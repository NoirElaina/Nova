use crate::llm::services::file_changes::{
    apply_patch_change, patch_paths, FileEditResult,
};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tauri::AppHandle;

pub(super) fn registrations() -> Vec<ToolRegistration> {
    vec![
        app_tool(
            apply_patch_tool,
            apply_patch_with_app,
            false,
            Some(apply_patch_permission),
        ),
    ]
}

fn apply_patch_tool() -> Tool {
    Tool {
        name: "apply_patch".into(),
        description: "Use the `apply_patch` tool to edit files. This is a FREEFORM tool — pass the raw patch string directly, do not wrap it in JSON. Format:\n*** Begin Patch\n*** Update File: /absolute/path\n@@ -start,count +start,count @@\n unchanged line\n-removed line\n+added line\n*** Add File: /absolute/path\n+line content\n*** Delete File: /absolute/path\n*** Move to: /old/path -> /new/path\n*** End Patch\nHunk headers MUST use unified diff format: @@ -oldStart,oldCount +newStart,newCount @@. Do NOT use Chinese line numbers like @@ 第3行. All file paths must be absolute.".to_string(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Patch text using *** Begin Patch / *** Update File / *** Add File / *** Delete File / @@ hunks / *** End Patch. Every file path in patch headers must be absolute. Hunk headers MUST use unified diff format: @@ -oldStart,oldCount +newStart,newCount @@. Example:\n*** Begin Patch\n*** Update File: /path/to/file.rs\n@@ -10,3 +10,4 @@\n line10\n+inserted line\n line11\n line12\n*** End Patch"
                }
            },
            "required": ["patch"]
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

fn result_json(result: FileEditResult) -> Result<ToolOutcome, ToolFailure> {
    let changed_files = result.files.len();
    Ok(ToolOutcome::json(json!({
        "ok": true,
        "files": result.files,
        "changed_files": changed_files,
        "change_batch_id": result.change_batch_id
    })))
}
