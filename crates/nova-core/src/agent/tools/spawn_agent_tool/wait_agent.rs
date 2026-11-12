use serde_json::{json, Value};
use std::time::Duration;

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        tool,
        execute_wait_agent_boxed,
        false,
        None,
        ToolDisclosure::Core,
    )]
}

pub fn tool() -> Tool {
    Tool {
        name: "wait_agent".into(),
        description: r#"Waits for a running subagent to complete or report progress, or waits for any subagent in the conversation if target is omitted.
Supports timeout_ms (default: 30000ms, max: 300000ms). Returns the subagent's execution status, outcome summary, and final findings."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Optional subagent ID or nickname to wait for. If omitted, waits for any subagent in this conversation."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Maximum time to wait in milliseconds (default 30000, min 1000, max 300000)."
                }
            }
        }),
    }
}

/// 宽松解析整型：部分非 OpenAI 模型会把 `timeout_ms` 序列化成 `30000.0` 或 `"30000"`，
/// 严格 `as_i64` 会让这些值静默回落到默认值。
fn parse_lenient_i64(value: Option<&Value>) -> Option<i64> {
    let v = value?;
    if let Some(i) = v.as_i64() {
        return Some(i);
    }
    if let Some(u) = v.as_u64() {
        return i64::try_from(u).ok();
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            return Some(f as i64);
        }
    }
    v.as_str()?.trim().parse::<i64>().ok()
}

fn execute_wait_agent_boxed(
    _core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let parent_id = conversation_id.clone().unwrap_or_else(|| "default".to_string());
        let target = input.get("target").and_then(Value::as_str);
        let timeout_ms = parse_lenient_i64(input.get("timeout_ms"))
            .unwrap_or(30_000)
            .clamp(1_000, 300_000) as u64;

        // 必须透传**父会话的**取消令牌：此前传 `CancellationToken::new()`，
        // 用户点停止后 wait_agent 仍会阻塞到超时。
        let parent_cancel =
            crate::agent::cancellation::get_token(conversation_id.as_deref());

        let wait_result = crate::agent::orchestration::registry::global_agent_registry()
            .wait(
                &parent_id,
                target,
                Duration::from_millis(timeout_ms),
                parent_cancel,
            )
            .await
            .map_err(ToolFailure::new)?;

        Ok(ToolOutcome::json(json!(wait_result)))
    })
}
