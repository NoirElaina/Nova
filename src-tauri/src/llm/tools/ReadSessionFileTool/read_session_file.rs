use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    // 只读工具，可批量并发。不需要权限审批——filename 经过 sanitize，
    // 路径由后端用当前会话 ID 自动拼接，AI 无法指定任意路径。
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "ReadSessionFile".into(),
        description: r#"Read a file that was uploaded to the current conversation session.

- `filename` is the name of the uploaded file (shown in the [Session Files] context section).
- Returns the file's text content.
- Only works for files uploaded in the current conversation; cannot read arbitrary filesystem paths (use Read for that).
- The session files list is injected into context as `[Session Files]` at the start of each turn."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The filename of the uploaded session file to read."
                }
            },
            "required": ["filename"],
            "additionalProperties": false
        }),
    }
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let filename = input
        .get("filename")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: filename"))?;

    let conv_id = conversation_id.ok_or_else(|| {
        ToolFailure::new("读取会话文件需要会话 ID，但当前无活跃会话".to_string())
    })?;

    let content = crate::llm::services::session_files::read_session_file(app, conv_id, filename)
        .map_err(ToolFailure::new)?;

    Ok(ToolOutcome::text(content))
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        execute_async(&app, conversation_id.as_deref(), input).await
    })
}
