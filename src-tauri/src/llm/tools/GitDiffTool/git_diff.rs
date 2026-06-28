use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    // 只读工具，无副作用，可批量并发执行，不需要权限审批。
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "GitDiff".into(),
        description: r#"Get the current workspace's uncommitted changes (git diff HEAD + untracked files).

Returns a structured text summary of all changed files with their change type (added/modified/deleted), additions/deletions count, and the actual diff content.

- No parameters needed: automatically resolves the conversation's workspace.
- Includes both staged and unstaged changes, plus untracked files.
- Use this instead of `git diff` via Bash when you need a clean, structured view of current changes without permission prompts.

The workspace's git status summary (branch + dirty file list + recent commits) is already injected into your context every turn via `[Project Context]`. Use this tool only when you need the actual diff content for specific files.
"#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    _input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let repo_root =
        crate::command::workspace::workspace_root_for_conversation(app, conversation_id)
            .map_err(|e| ToolFailure::new(format!("Failed to resolve workspace: {}", e)))?;

    if !crate::llm::services::git_ops::is_repo_initialized(&repo_root) {
        return Ok(ToolOutcome::text(
            "Workspace is not a git repository. No diff available.",
        ));
    }

    let diff = crate::llm::services::git_ops::collect_workspace_diff(&repo_root)
        .map_err(|e| ToolFailure::new(format!("Failed to collect workspace diff: {}", e)))?;

    if diff.files.is_empty() {
        return Ok(ToolOutcome::text("Working tree is clean. No changes to show."));
    }

    let output = render_workspace_diff(&diff);
    Ok(ToolOutcome::text(output))
}

fn render_workspace_diff(
    diff: &crate::llm::services::git_ops::WorkspaceDiff,
) -> String {
    let mut out = format!(
        "Workspace diff: {} file(s) changed, +{} / -{}\n\n",
        diff.files.len(),
        diff.total_additions,
        diff.total_deletions
    );

    for file in &diff.files {
        out.push_str(&format!(
            "--- {} ({}, +{}/-{}) ---\n",
            file.path, file.change_type, file.additions, file.deletions
        ));
        // 限制每文件 diff 行数，避免单个超大 diff 爆上下文。
        let max_lines = 200usize;
        let total = file.diff.len();
        for line in file.diff.iter().take(max_lines) {
            let prefix = match line.kind.as_str() {
                "add" => "+",
                "remove" => "-",
                _ => " ",
            };
            out.push_str(&format!("{}{}\n", prefix, line.text));
        }
        if total > max_lines {
            out.push_str(&format!(
                "... ({} more diff lines truncated)\n",
                total - max_lines
            ));
        }
        out.push('\n');
    }

    out
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
