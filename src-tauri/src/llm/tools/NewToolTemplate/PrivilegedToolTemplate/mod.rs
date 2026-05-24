use crate::llm::tools::{
    app_tool_with_extras, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::{Message, Tool};
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool_with_extras(
        tool,
        execute_with_app_boxed,
        false,
        Some(permission),
        Some(postprocess_output),
    )
}

pub fn tool() -> Tool {
    Tool {
        name: "privileged_tool".into(),
        description: "A template for tools that need permission and/or side-channel output.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "The sensitive target or action subject"
                }
            },
            "required": ["target"]
        }),
    }
}

pub async fn execute_with_app(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> String {
    let target = match input.get("target").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'target'" }).to_string(),
    };

    json!({
        "ok": true,
        "target": target,
        "message": "Replace this with your actual sensitive operation."
    })
    .to_string()
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let target = input
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .trim();

    Some(ToolPermissionDescriptor {
        signature: format!("privileged_tool:{}", target),
        preview: format!("执行 privileged_tool：{}", target),
        warning: Some("这是一个需要授权的模板工具，请根据真实风险调整提示。".to_string()),
        needs_approval: true,
    })
}

pub fn postprocess_output(output: &str) -> (String, Vec<Message>) {
    (output.to_string(), Vec::new())
}
