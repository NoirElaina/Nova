use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 返回 StructuredOutput 工具的注册信息。
// 这个工具只包装并返回 JSON，不读写外部状态。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回模型可见的 StructuredOutput 元数据。
// schema 故意保持宽松，允许模型直接返回任意结构化 JSON。
pub fn tool() -> Tool {
    Tool {
        name: "StructuredOutput".into(),
        description: "Return structured JSON output as the final machine-readable result.".into(),
        input_schema: json!({
            "type": "object",
            "description": "Arbitrary structured JSON object that will be returned to the caller as-is.",
            "properties": {},
            "additionalProperties": true
        }),
    }
}

// 把输入 JSON 原样挂到 `structured_output` 字段里返回。
// `input` 就是模型要作为最终机器可读结果交付给调用方的对象。
fn execute_local(input: Value) -> String {
    json!({
        "ok": true,
        "message": "Structured output provided successfully",
        "structured_output": input
    })
    .to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_local(input) })
}
