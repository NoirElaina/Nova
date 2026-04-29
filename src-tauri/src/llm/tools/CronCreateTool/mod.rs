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
        name: "CronCreate".into(),
        description: "Schedule a recurring or one-shot prompt task.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "cron": { "type": "string", "description": "5-field cron expression: M H DoM Mon DoW" },
                "prompt": { "type": "string", "description": "Prompt payload to run on schedule" },
                "recurring": { "description": "true (default) for recurring schedule, false for one-shot" },
                "durable": { "description": "true to persist in app_data_dir across restarts" }
            },
            "required": ["cron", "prompt"]
        }),
    }
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "CronCreate requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

fn parse_semantic_bool(input: &Value, key: &str, default_value: bool) -> bool {
    let Some(value) = input.get(key) else {
        return default_value;
    };

    if let Some(v) = value.as_bool() {
        return v;
    }

    if let Some(v) = value.as_i64() {
        return v != 0;
    }

    if let Some(v) = value.as_u64() {
        return v != 0;
    }

    if let Some(v) = value.as_str() {
        let lower = v.trim().to_ascii_lowercase();
        return matches!(lower.as_str(), "1" | "true" | "yes" | "on");
    }

    default_value
}

fn parse_field_range(index: usize) -> (u32, u32) {
    match index {
        0 => (0, 59),
        1 => (0, 23),
        2 => (1, 31),
        3 => (1, 12),
        4 => (0, 7),
        _ => (0, 0),
    }
}

fn parse_number_in_range(raw: &str, min: u32, max: u32) -> bool {
    raw.parse::<u32>()
        .ok()
        .map(|v| v >= min && v <= max)
        .unwrap_or(false)
}

fn validate_cron_segment(segment: &str, min: u32, max: u32) -> bool {
    if segment.is_empty() {
        return false;
    }

    let (base, step) = match segment.split_once('/') {
        Some((base, step)) => (base, Some(step)),
        None => (segment, None),
    };

    if let Some(step_raw) = step {
        let valid_step = step_raw
            .parse::<u32>()
            .ok()
            .map(|v| v > 0)
            .unwrap_or(false);
        if !valid_step {
            return false;
        }
    }

    if base == "*" {
        return true;
    }

    if let Some((start, end)) = base.split_once('-') {
        let valid_start = parse_number_in_range(start, min, max);
        let valid_end = parse_number_in_range(end, min, max);
        if !valid_start || !valid_end {
            return false;
        }
        let s = start.parse::<u32>().ok().unwrap_or(0);
        let e = end.parse::<u32>().ok().unwrap_or(0);
        return s <= e;
    }

    parse_number_in_range(base, min, max)
}

fn validate_cron_field(field: &str, min: u32, max: u32) -> bool {
    field
        .split(',')
        .all(|segment| validate_cron_segment(segment.trim(), min, max))
}

fn validate_cron_expression(expr: &str) -> Result<(), String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err("Cron expression must contain exactly 5 fields: M H DoM Mon DoW".to_string());
    }

    for (index, field) in fields.iter().enumerate() {
        let (min, max) = parse_field_range(index);
        if !validate_cron_field(field.trim(), min, max) {
            return Err(format!(
                "Invalid cron field {}='{}'. Expected range {}-{} with optional *, -, /, ,",
                index + 1,
                field,
                min,
                max
            ));
        }
    }

    Ok(())
}

pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let cron = match input.get("cron").and_then(|v| v.as_str()).map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => return json!({ "ok": false, "error": "CronCreate requires non-empty 'cron'" }).to_string(),
    };

    if let Err(e) = validate_cron_expression(cron) {
        return json!({ "ok": false, "error": e }).to_string();
    }

    let prompt = match input.get("prompt").and_then(|v| v.as_str()).map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => return json!({ "ok": false, "error": "CronCreate requires non-empty 'prompt'" }).to_string(),
    };

    let recurring = parse_semantic_bool(&input, "recurring", true);
    let durable = parse_semantic_bool(&input, "durable", false);

    match crate::command::cron::create_scheduled_task(
        app.clone(),
        cron.to_string(),
        prompt.to_string(),
        Some(recurring),
        Some(durable),
    )
    .await
    {
        Ok(saved) => json!({
            "ok": true,
            "id": saved.id,
            "cron": saved.cron,
            "humanSchedule": saved.cron,
            "prompt": saved.prompt,
            "conversationId": saved.conversation_id,
            "recurring": saved.recurring,
            "durable": saved.durable,
            "createdAt": saved.created_at
        })
        .to_string(),
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    }
}
