use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::thread;
use std::time::Duration;

const MAX_SLEEP_MS: u64 = 5 * 60 * 1000;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "Sleep".into(),
        description: "Wait for a specified duration without occupying a shell process.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "duration_ms": { "type": "integer", "description": "Sleep duration in milliseconds" },
                "ms": { "type": "integer", "description": "Alias of duration_ms" },
                "duration_seconds": { "type": "number", "description": "Sleep duration in seconds" },
                "seconds": { "type": "number", "description": "Alias of duration_seconds" }
            }
        }),
    }
}

fn parse_positive_u64(value: &Value) -> Option<u64> {
    if let Some(v) = value.as_u64() {
        return (v > 0).then_some(v);
    }
    if let Some(v) = value.as_i64() {
        return (v > 0).then_some(v as u64);
    }
    if let Some(v) = value.as_f64() {
        if v.is_finite() && v > 0.0 {
            return Some(v.round() as u64);
        }
    }
    if let Some(v) = value.as_str() {
        let parsed = v.trim().parse::<u64>().ok()?;
        return (parsed > 0).then_some(parsed);
    }
    None
}

fn parse_sleep_ms(input: &Value) -> Option<u64> {
    if let Some(v) = input.get("duration_ms").and_then(parse_positive_u64) {
        return Some(v);
    }

    if let Some(v) = input.get("ms").and_then(parse_positive_u64) {
        return Some(v);
    }

    if let Some(v) = input.get("duration_seconds").and_then(parse_positive_u64) {
        return Some(v.saturating_mul(1000));
    }

    if let Some(v) = input.get("seconds").and_then(parse_positive_u64) {
        return Some(v.saturating_mul(1000));
    }

    None
}

pub fn execute(input: Value) -> String {
    let requested_ms = match parse_sleep_ms(&input) {
        Some(v) => v,
        None => {
            return json!({
                "ok": false,
                "error": "Sleep requires one of: duration_ms | ms | duration_seconds | seconds (positive number)"
            })
            .to_string();
        }
    };

    let slept_ms = requested_ms.min(MAX_SLEEP_MS);
    thread::sleep(Duration::from_millis(slept_ms));

    json!({
        "ok": true,
        "requested_ms": requested_ms,
        "slept_ms": slept_ms,
        "clamped": requested_ms > MAX_SLEEP_MS
    })
    .to_string()
}
