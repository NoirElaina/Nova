use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 TaskStop，声明它是写类会话工具，用于停止当前会话里的任务。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回暴露给模型的工具元数据。
// 当前接口统一使用 `task_id` 指定要停止的任务。
pub fn tool() -> Tool {
    Tool {
        name: "TaskStop".into(),
        description: "Stop a running task by id.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "task_id": { "type": ["string", "integer"] }
            },
            "required": ["task_id"]
        }),
    }
}

// 读取 `task_id`，并把字符串或整数统一解析成内部任务 id。
fn parse_task_id(input: &Value) -> Option<u64> {
    if let Some(v) = input.get("task_id") {
        if let Some(id) = v.as_u64() {
            return Some(id);
        }
        if let Some(s) = v.as_str() {
            if let Ok(id) = s.trim().parse::<u64>() {
                return Some(id);
            }
        }
    }
    None
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_scoped(conversation_id.as_deref(), input) })
}

fn execute_scoped(conversation_id: Option<&str>, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let Some(task_id) = parse_task_id(&input) else {
        return Err(ToolFailure::invalid_input("Missing 'task_id'"));
    };

    match task_store::update(conversation_id, task_id, None, Some("stopped".into()), None) {
        Some(task) => Ok(ToolOutcome::json(json!({
            "ok": true,
            "message": format!("Successfully stopped task: {}", task.id),
            "task_id": task.id.to_string(),
            "task_type": "todo",
            "command": task.title
        }))),
        None => Err(ToolFailure::new(format!("Task id {} not found", task_id))),
    }
}
