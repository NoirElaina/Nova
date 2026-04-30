use crate::llm::tools::shared::task_store;
use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

// 注册 TaskStop，声明它是写类同步工具，用于停止当前会话里的任务。
pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false, None)
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

// 把目标任务状态更新为 stopped，并返回停止结果。
pub fn execute(input: Value) -> String {
    let Some(task_id) = parse_task_id(&input) else {
        return "Error: Missing 'task_id'".into();
    };

    match task_store::update(task_id, None, Some("stopped".into()), None) {
        Some(task) => json!({
            "ok": true,
            "message": format!("Successfully stopped task: {}", task.id),
            "task_id": task.id.to_string(),
            "task_type": "todo",
            "command": task.title
        })
        .to_string(),
        None => format!("Error: Task id {} not found", task_id),
    }
}
