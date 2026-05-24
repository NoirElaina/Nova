use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::AppHandle;

const MAX_SLEEP_MS: u64 = 5 * 60 * 1000;

// 返回 Sleep 工具的注册信息。
// 这个工具只等待时间流逝，不修改任何外部状态，所以标成只读。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回模型可见的 Sleep 元数据。
pub fn tool() -> Tool {
    Tool {
        name: "Sleep".into(),
        description: "Wait for a specified duration without occupying a shell process.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "duration_ms": { "type": "integer", "description": "Sleep duration in milliseconds" }
            },
            "required": ["duration_ms"]
        }),
    }
}

fn parse_positive_u64(value: &Value) -> Option<u64> {
    if let Some(v) = value.as_u64() { return (v > 0).then_some(v); }
    if let Some(v) = value.as_i64() { return (v > 0).then_some(v as u64); }
    if let Some(v) = value.as_f64() {
        if v.is_finite() && v > 0.0 { return Some(v.round() as u64); }
    }
    if let Some(v) = value.as_str() {
        let parsed = v.trim().parse::<u64>().ok()?;
        return (parsed > 0).then_some(parsed);
    }
    None
}

fn parse_sleep_ms(input: &Value) -> Option<u64> {
    input.get("duration_ms").and_then(parse_positive_u64)
}

// 分块异步 sleep，每 50ms 检查一次取消标记，保证用户取消可以中断等待。
async fn execute_async(conversation_id: Option<&str>, input: Value) -> String {
    let requested_ms = match parse_sleep_ms(&input) {
        Some(v) => v,
        None => {
            return json!({
                "ok": false,
                "error": "Sleep requires positive integer 'duration_ms'"
            })
            .to_string();
        }
    };
    let slept_ms = requested_ms.min(MAX_SLEEP_MS);
    let chunk_ms: u64 = 50;
    let mut elapsed: u64 = 0;
    while elapsed < slept_ms {
        if crate::llm::cancellation::is_cancelled(conversation_id) {
            return json!({ "ok": false, "error": "cancelled", "slept_ms": elapsed }).to_string();
        }
        let to_sleep = (slept_ms - elapsed).min(chunk_ms);
        tokio::time::sleep(Duration::from_millis(to_sleep)).await;
        elapsed += to_sleep;
    }
    json!({
        "ok": true,
        "requested_ms": requested_ms,
        "slept_ms": slept_ms,
        "clamped": requested_ms > MAX_SLEEP_MS
    })
    .to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(conversation_id.as_deref(), input).await })
}


