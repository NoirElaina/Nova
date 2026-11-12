//! 原生 Agent 差异补丁工具实现。

use serde_json::{json, Value};

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

use super::apply::execute_patch_hunks;
use super::parser::parse_patch;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        execute_with_app_boxed,
        false,
        Some(permission),
        ToolDisclosure::Core,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "apply_patch".into(),
        description: r#"Applies unified diff patches to one or more files in the workspace.
Provides surgical and batch code edits with high reliability.

## Format Grammar
Every patch must start with `*** Begin Patch` and end with `*** End Patch`.

Supported operations:
1. Update existing file:
*** Begin Patch
*** Update File: src/main.rs
@@ context @@
-old line
+new line
*** End Patch

2. Add new file:
*** Begin Patch
*** Add File: path/to/new_file.txt
+first line
+second line
*** End Patch

3. Delete file:
*** Begin Patch
*** Delete File: path/to/old_file.txt
*** End Patch

4. Move/rename file:
*** Begin Patch
*** Update File: old_path.rs
*** Move to: new_path.rs
@@ @@
-old content
+new content
*** End Patch

## Rules
- Context lines (unchanged) start with a space ' ' or empty line.
- Removed lines start with '-'.
- Added lines start with '+'.
- Lines with @@ denote context anchors (such as function/class signatures).
- Multiple files can be patched in a single invocation."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "The patch content starting with '*** Begin Patch' and ending with '*** End Patch'"
                }
            },
            "required": ["patch"]
        }),
    }
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let patch_str = input.get("patch").and_then(Value::as_str).unwrap_or("");
    let Ok(args) = parse_patch(patch_str) else {
        // 解析失败时无法判断目标路径，按最高风险处理，交由审批拦下。
        return Some(ToolPermissionDescriptor {
            signature: "apply_patch:<unparsed>".to_string(),
            preview: "无法解析的差异补丁".to_string(),
            warning: Some("补丁格式非法，无法静态确定写入目标。".to_string()),
            risk: crate::agent::permissions::RiskLevel::Risky,
        });
    };
    if args.hunks.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: "apply_patch:<empty>".to_string(),
            preview: "空补丁".to_string(),
            warning: Some("补丁不包含任何文件操作。".to_string()),
            risk: crate::agent::permissions::RiskLevel::Forbidden,
        });
    }

    // 关键：必须遍历**全部** hunk 及其源/目标路径（含 Move to 源路径），
    // 防止借首个良性 hunk 或 move 源路径绕过受保护路径拦截。
    let mut worst = crate::agent::permissions::RiskLevel::Safe;
    let mut warning: Option<String> = None;
    let mut all_paths: Vec<String> = Vec::new();
    for hunk in &args.hunks {
        for path in hunk.paths() {
            let path_str = path.to_string_lossy().to_string();
            if warning.is_none() {
                if let Err(reason) = crate::agent::permissions::checks::check_file_path(&path_str) {
                    warning = Some(reason);
                    worst = crate::agent::permissions::RiskLevel::Forbidden;
                }
            }
            if !all_paths.contains(&path_str) {
                all_paths.push(path_str);
            }
        }
    }

    let mut norm_paths: Vec<String> = all_paths
        .iter()
        .map(|p| crate::agent::permissions::checks::normalize_path_for_match(p))
        .collect();
    norm_paths.sort();
    norm_paths.dedup();

    let preview_summary: Vec<&str> = all_paths.iter().take(3).map(String::as_str).collect();
    let preview_suffix = if all_paths.len() > 3 {
        format!(" 等 {} 处路径", all_paths.len())
    } else {
        String::new()
    };
    Some(ToolPermissionDescriptor {
        signature: format!("apply_patch:{}", norm_paths.join("|")),
        preview: format!(
            "应用差异补丁（{} 个操作）：{}{}",
            args.hunks.len(),
            crate::agent::permissions::checks::truncate_chars(&preview_summary.join(", "), 160),
            preview_suffix
        ),
        warning,
        risk: worst,
    })
}

async fn execute_async(
    core: &Core,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let patch = input
        .get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: patch"))?;

    let parsed = parse_patch(patch).map_err(|e| ToolFailure::new(e.to_string()))?;
    // 补丁必须相对**会话工作区**解析，而不是 GUI/daemon 进程的 cwd：
    // Tauri 进程的 cwd 是 exe 所在目录，拿它当基准会把补丁写到错误的地方。
    let workspace = crate::services::workspace::workspace_root_for_conversation(core, conversation_id)
        .map_err(|e| ToolFailure::new(format!("Failed to resolve workspace: {e}")))?;

    let report = execute_patch_hunks(&workspace, &parsed.hunks)
        .await
        .map_err(ToolFailure::new)?;

    Ok(ToolOutcome::json(report.to_json()))
}

fn execute_with_app_boxed(
    core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        execute_async(&core, conversation_id.as_deref(), input).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：权限只校验第一个 hunk 时，可以用良性文件骗过审批，
    /// 随后在第二个 hunk 里写受保护路径（/etc 在 Windows 与 Unix 都受保护）。
    #[test]
    fn permission_scans_every_hunk() {
        let patch = "*** Begin Patch\n*** Add File: notes.md\n+hi\n*** Add File: /etc/nova_evil.conf\n+pwn\n*** End Patch";
        let descriptor = permission(&json!({ "patch": patch })).expect("descriptor");
        assert_eq!(
            descriptor.risk,
            crate::agent::permissions::RiskLevel::Forbidden,
            "warning={:?} preview={}",
            descriptor.warning,
            descriptor.preview
        );
    }

    #[test]
    fn permission_marks_unparsable_patch_as_risky() {
        let descriptor = permission(&json!({ "patch": "not a patch" })).expect("descriptor");
        assert_eq!(
            descriptor.risk,
            crate::agent::permissions::RiskLevel::Risky
        );
    }

    #[test]
    fn permission_scans_move_source_path_and_includes_all_paths_in_signature() {
        let evil_move = "*** Begin Patch\n*** Update File: /etc/hosts\n*** Move to: safe.txt\n@@\n-a\n+b\n*** End Patch";
        let desc = permission(&json!({ "patch": evil_move })).expect("descriptor");
        assert_eq!(desc.risk, crate::agent::permissions::RiskLevel::Forbidden);

        let p1 = "*** Begin Patch\n*** Add File: a.txt\n+1\n*** Add File: b.txt\n+2\n*** End Patch";
        let p2 = "*** Begin Patch\n*** Add File: a.txt\n+1\n*** Add File: c.txt\n+3\n*** End Patch";
        let d1 = permission(&json!({ "patch": p1 })).expect("d1");
        let d2 = permission(&json!({ "patch": p2 })).expect("d2");
        assert_ne!(d1.signature, d2.signature, "distinct multi-file patches must not share signature");
    }
}
