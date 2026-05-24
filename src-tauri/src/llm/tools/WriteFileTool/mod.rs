use crate::llm::services::file_changes::FileChangeDraft;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(
        tool,
        execute,
        execute_with_app_boxed,
        false,
        Some(permission),
    )
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    // 写文件属于高风险操作，由工具自己声明“写到哪儿”。
    crate::llm::utils::permissions::describe_file_write_permission(
        "write_file",
        "文件写入",
        "path",
        input,
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "write_file".into(),
        description: "Write content to a file inside the conversation WorkspaceRoot. This completely overwrites the file and records the change for review.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative path or absolute path inside WorkspaceRoot" },
                "content": { "type": "string", "description": "The content to write" }
            },
            "required": ["path", "content"]
        }),
    }
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "error": "write_file requires AppHandle-aware execution inside a conversation WorkspaceRoot."
    })
    .to_string()
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

async fn execute_async(app: &AppHandle, conversation_id: Option<&str>, input: Value) -> String {
    let path = match input.get("path").and_then(|v| v.as_str()) {
        Some(path) if !path.trim().is_empty() => path,
        _ => return json!({ "ok": false, "error": "Missing 'path' argument" }).to_string(),
    };
    let content = match input.get("content").and_then(|v| v.as_str()) {
        Some(content) => content,
        _ => return json!({ "ok": false, "error": "Missing 'content' argument" }).to_string(),
    };
    let root =
        match crate::command::workspace::workspace_root_for_conversation(app, conversation_id) {
            Ok(root) => root,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
    let target = match crate::llm::services::file_changes::resolve_tool_path(&root, path) {
        Ok(path) => path,
        Err(error) => return json!({ "ok": false, "error": error }).to_string(),
    };
    let before = if target.exists() {
        match fs::read_to_string(&target) {
            Ok(content) => Some(content),
            Err(error) => {
                return json!({
                    "ok": false,
                    "error": format!("Cannot safely capture existing file before write_file: {}", error)
                })
                .to_string()
            }
        }
    } else {
        None
    };

    if let Some(parent) = target.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return json!({ "ok": false, "error": format!("Error creating parent directory: {}", error) })
                .to_string();
        }
    }
    if let Err(error) = fs::write(&target, content) {
        return json!({ "ok": false, "error": format!("Error writing file: {}", error) }).to_string();
    }

    let change_batch_id = match crate::llm::services::file_changes::record_change_batch(
        app,
        conversation_id,
        &root,
        "write_file",
        vec![FileChangeDraft {
            path: target.clone(),
            before,
            after: Some(content.to_string()),
        }],
    ) {
        Ok(batch_id) => batch_id,
        Err(error) => {
            return json!({
                "ok": true,
                "message": "Successfully wrote to file",
                "files": [path],
                "changed_files": 1,
                "change_batch_id": null,
                "review_error": error
            })
            .to_string()
        }
    };

    json!({
        "ok": true,
        "message": "Successfully wrote to file",
        "files": [path],
        "changed_files": 1,
        "change_batch_id": change_batch_id
    })
    .to_string()
}
