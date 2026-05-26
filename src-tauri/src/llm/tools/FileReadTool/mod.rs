use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "read_file".into(),
        description: "Read the content of a file inside the conversation WorkspaceRoot.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative path or absolute path inside WorkspaceRoot" }
            },
            "required": ["path"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let path = match input.get("path").and_then(|v| v.as_str()) {
        Some(path) if !path.trim().is_empty() => path,
        _ => return Err(ToolFailure::invalid_input("Missing 'path' argument")),
    };
    let root =
        match crate::command::workspace::workspace_root_for_conversation(app, conversation_id) {
            Ok(root) => root,
            Err(error) => return Err(ToolFailure::new(error)),
        };
    let target = match crate::llm::services::file_changes::resolve_tool_path(&root, path) {
        Ok(path) => path,
        Err(error) => return Err(ToolFailure::new(error)),
    };
    if !target.is_file() {
        return Err(ToolFailure::new("path is not a file"));
    }

    fs::read_to_string(&target)
        .map(ToolOutcome::text)
        .map_err(|error| ToolFailure::new(format!("Error reading file: {}", error)))
}
