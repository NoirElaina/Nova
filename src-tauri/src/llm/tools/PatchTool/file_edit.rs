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
        description: "FREEFORM tool -- pass raw patch string directly.\n\nFormat:\n*** Begin Patch\n*** Add File: /absolute/path/to/file\n+line1\n+line2\n*** Update File: /absolute/path/to/file\n@@\n context line\n+added line\n-removed line\n@@\n next context\n*** Delete File: /absolute/path/to/file\n*** End Patch\n\nRules:\n- Paths must be absolute\n- @@ is a section separator (no line numbers needed)\n- Lines starting with space = context (must match file exactly)\n- Lines starting with + = add\n- Lines starting with - = remove\n- Every Add File line must start with +".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Raw patch text **MUST** starting with *** Begin Patch and ending with *** End Patch"
                }
            }
        }),
    }
}

fn apply_patch_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let patch = match &input {
            Value::String(s) => s.as_str(),
            Value::Object(obj) => obj.get("patch").and_then(Value::as_str).unwrap_or(""),
            _ => return Err(ToolFailure::invalid_input("apply_patch requires raw patch text or {patch: ...}")),
        };
        match apply_patch_change(&app, conversation_id.as_deref(), patch).await {
            Ok(result) => result_json(result),
            Err(error) => Err(ToolFailure::new(error)),
        }
    })
}

fn apply_patch_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let patch = match input {
        Value::String(s) => s.as_str(),
        Value::Object(obj) => obj.get("patch").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    };
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
