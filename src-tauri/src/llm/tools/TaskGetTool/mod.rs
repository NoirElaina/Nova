use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 TaskGet，声明它是只读同步工具，用于按 id 读取当前会话任务。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true, None)
}

// 返回暴露给模型的工具元数据。
// 当前接口统一使用 `task_id`。
pub fn tool() -> Tool {
    Tool {
        name: "TaskGet".into(),
        description: "Retrieve a task by ID.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "task_id": { "type": ["integer", "string"] }
            },
            "required": ["task_id"]
        }),
    }
}

// 读取 `task_id`，并把字符串或整数统一解析成内部任务 id。
fn parse_task_id(input: &Value) -> Option<u64> {
    if let Some(id) = input.get("task_id") {
        if let Some(v) = id.as_u64() {
            return Some(v);
        }
        if let Some(s) = id.as_str() {
            return s.trim().parse::<u64>().ok();
        }
    }
    None
}

// 根据解析出的 task_id 读取任务，并把任务对象原样放进 JSON 结果里。
pub fn execute(_input: Value) -> String {
    json!({ "ok": false, "error": "TaskGet requires conversation-aware execution" }).to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_scoped(conversation_id.as_deref(), input) })
}

fn execute_scoped(conversation_id: Option<&str>, input: Value) -> String {
    let Some(task_id) = parse_task_id(&input) else {
        return json!({ "ok": false, "error": "Missing 'task_id'" }).to_string();
    };

    let task = task_store::get(conversation_id, task_id);
    json!({ "ok": true, "task": task }).to_string()
}
