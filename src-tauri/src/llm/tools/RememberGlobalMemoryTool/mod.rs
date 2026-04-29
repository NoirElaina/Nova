use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "remember_global_memory".into(),
        description: "Persist a stable cross-session memory item (preference/fact/rule) without user confirmation.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Memory content to persist globally"
                },
                "kind": {
                    "type": "string",
                    "enum": ["preference", "fact", "rule"],
                    "description": "Memory type classification"
                },
                "source": {
                    "type": "string",
                    "description": "Optional source tag, default assistant"
                }
            },
            "required": ["content"]
        }),
    }
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "remember_global_memory requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let content = match input.get("content").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return "Error: Missing non-empty 'content' argument".into(),
    };

    let kind = input.get("kind").and_then(|v| v.as_str());
    let source = input.get("source").and_then(|v| v.as_str());

    match crate::llm::history::upsert_global_memory(app, content, kind, source).await {
        Ok(entry) => json!({ "ok": true, "memory": entry }).to_string(),
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    }
}
