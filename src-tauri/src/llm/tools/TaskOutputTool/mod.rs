use crate::llm::tools::shared::task_store;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 注册 TaskOutput，声明它是只读会话工具，用于查询任务输出摘要。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回暴露给模型的工具元数据。
// 当前接口统一使用 `task_id` 指定要读取的任务。
pub fn tool() -> Tool {
    Tool {
        name: "TaskOutput".into(),
        description: "Return task output-style summary by task id.".into(),
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

fn execute_scoped(conversation_id: Option<&str>, input: Value) -> String {
    let Some(task_id) = parse_task_id(&input) else {
        return json!({ "ok": false, "error": "Missing 'task_id'" }).to_string();
    };

    let Some(task) = task_store::get(conversation_id, task_id) else {
        return json!({ "ok": true, "retrieval_status": "not_found", "task": Value::Null }).to_string();
    };

    // output: 拼给模型看的多行任务摘要文本。
    let output = format!(
        "Task #{}\nTitle: {}\nStatus: {}\nNotes: {}",
        task.id,
        task.title,
        task.status,
        task.notes.clone().unwrap_or_else(|| "(none)".into())
    );

    json!({
        "ok": true,
        "retrieval_status": "success",
        "task": {
            "task_id": task.id.to_string(),
            "task_type": "todo",
            "status": task.status,
            "description": task.title,
            "output": output
        }
    })
    .to_string()
}
