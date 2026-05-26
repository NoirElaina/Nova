use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use chrono::Local;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把 async 的调度创建逻辑包装成统一 future。
// `input` 里会携带 cron、prompt、recurring、durable 这些创建任务所需参数。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 CronCreate 的注册信息。
// 这里声明 `read_only=false`，因为它会创建新的计划任务记录。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的 CronCreate 元数据。
// 模型通过 schema 知道要提供 cron 表达式和 prompt 内容。
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

// 读取 input[key] 并把布尔语义统一成 true/false。
// `default_value` 表示字段缺失或无法解析时应该落到哪个默认值。
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

// 创建计划任务并返回保存结果。
// `cron` 是触发表达式，`prompt` 是到时要执行的提示词，`recurring/durable` 控制重复和持久化行为。
async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let cron = match input.get("cron").and_then(|v| v.as_str()).map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => return Err(ToolFailure::invalid_input("CronCreate requires non-empty 'cron'")),
    };

    let schedule_info = match crate::llm::services::cron_schedule::schedule_info(cron, &Local::now()) {
        Ok(info) => info,
        Err(e) => return Err(ToolFailure::invalid_input(e)),
    };

    let prompt = match input.get("prompt").and_then(|v| v.as_str()).map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => return Err(ToolFailure::invalid_input("CronCreate requires non-empty 'prompt'")),
    };

    // recurring: true 表示反复执行；false 表示一次性任务。
    let recurring = parse_semantic_bool(&input, "recurring", true);
    // durable: true 表示写入持久化存储，应用重启后仍然保留。
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
        Ok(saved) => Ok(ToolOutcome::json(json!({
            "ok": true,
            "id": saved.id,
            "cron": saved.cron,
            "humanSchedule": schedule_info.human_schedule,
            "nextRunAt": schedule_info.next_run_at,
            "prompt": saved.prompt,
            "conversationId": saved.conversation_id,
            "recurring": saved.recurring,
            "durable": saved.durable,
            "createdAt": saved.created_at
        }))),
        Err(e) => Err(ToolFailure::new(e)),
    }
}
