use crate::llm::tools::{
    app_tool, app_tool_with_extras, sync_tool, AppExecuteFuture, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::{Message, Tool};
use serde_json::{json, Value};
use tauri::AppHandle;

// Copy this file into a new tool folder, then rename:
// - NewToolTemplate -> YourToolName
// - new_tool -> your real tool name
//
// See README.md in this folder for the recommended workflow and the smaller
// template variants (read-only / app-aware / privileged).
//
// After copying, add one line to the `declare_builtin_tools!` list in
// src-tauri/src/llm/tools/mod.rs:
// - your_tool => "YourTool/mod.rs",

pub fn tool() -> Tool {
    Tool {
        name: "new_tool".into(),
        description: "Describe what this tool does in one sentence.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Example input field"
                }
            },
            "required": ["input"]
        }),
    }
}

// Use this for pure sync tools with no AppHandle dependency.
pub fn execute(input: Value) -> String {
    let value = match input.get("input").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'input'" }).to_string(),
    };

    json!({
        "ok": true,
        "echo": value
    })
    .to_string()
}

// Keep this only when the tool needs AppHandle, async work, or conversation scope.
pub async fn execute_with_app(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> String {
    execute(input)
}

// Bridge async execution into the registry metadata.
fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

// Optional: return side-channel messages such as screenshots, extra context, etc.
// Delete this if your tool only returns normal JSON/text output.
pub fn postprocess_output(output: &str) -> (String, Vec<Message>) {
    (output.to_string(), Vec::new())
}

// Optional: declare permission behavior here instead of editing permissions/mod.rs.
// Delete this if the tool does not need a custom permission gate.
fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let preview = input
        .get("input")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .trim();

    Some(ToolPermissionDescriptor {
        signature: format!("new_tool:{}", preview),
        preview: format!("执行 new_tool：{}", preview),
        warning: Some("这个工具可能会执行敏感操作，请确认后再授权。".to_string()),
        needs_approval: true,
    })
}

pub(crate) fn registration() -> ToolRegistration {
    // Pick one pattern and delete the others:

    // 1. Pure sync tool:
    // sync_tool(tool, execute, true)

    // 2. App-aware tool:
    // app_tool(tool, execute, execute_with_app_boxed, false)

    // 3. App-aware tool with permission/postprocess:
    app_tool_with_extras(
        tool,
        execute,
        execute_with_app_boxed,
        false,
        Some(permission),
        Some(postprocess_output),
    )
}
