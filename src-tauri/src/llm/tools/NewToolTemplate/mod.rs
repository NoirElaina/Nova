use crate::llm::tools::{
    app_tool_with_extras, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::{Message, Tool};
use serde_json::{json, Value};
use tauri::AppHandle;

// 这是当前的全功能模板。
// 复制后只保留这一条最新路径：AppHandle-aware 执行 + 显式权限声明 + 可选后处理。
// 如果你的工具更简单，再去用子模板目录里的只读版或 App 版。

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

pub fn execute(input: Value) -> String {
    json!({
        "ok": false,
        "message": "new_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app.",
        "input": input
    })
    .to_string()
}

pub async fn execute_with_app(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> String {
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

// 把 async 执行函数桥接成注册表需要的 future 形态。
fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

// 可选后处理：需要 side-channel 消息时在这里补。
pub fn postprocess_output(output: &str) -> (String, Vec<Message>) {
    (output.to_string(), Vec::new())
}

// 显式权限声明：敏感工具在这里返回稳定的签名和提示文案。
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
    app_tool_with_extras(
        tool,
        execute,
        execute_with_app_boxed,
        false,
        Some(permission),
        Some(postprocess_output),
    )
}
