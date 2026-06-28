use crate::llm::tools::shared::todo_state::{global_registry, TodoEntry};
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub(super) fn registration() -> ToolRegistration {
    // TodoWrite 是内部状态工具，无副作用，不需要权限审批，但也不是只读
    // （它修改状态），所以 read_only=false 避免被批量并发执行。
    app_tool(tool, execute_with_app_boxed, false, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "TodoWrite".into(),
        description: r#"Create and manage a structured task list for the current conversation.

Use this tool when working on complex, multi-step tasks (3+ distinct steps) to:
- Track progress on planned work
- Show the user what's been done and what's remaining
- Avoid losing track of subtasks during long tool chains

The list is per-conversation: each `todos` call replaces the entire list. Always send the FULL updated list, not just changes.

Guidelines:
- Create todos for tasks with 3+ steps or when the user gives a multi-part request.
- Mark a todo `in_progress` BEFORE starting work on it, `completed` when done.
- Keep exactly ONE todo `in_progress` at a time (or zero between steps).
- Use `pending` for not-yet-started items.
- Skip this tool for trivial 1-2 step tasks — just do the work directly.

Each todo needs: `content` (imperative, e.g. "Add input validation to login form"), `status` (pending|in_progress|completed), `priority` (high|medium|low).
"#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Full replacement list of todos. Send the entire list every call.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "The task description in imperative voice (e.g. 'Add input validation')"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Current state of this task"
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["high", "medium", "low"],
                                "description": "Importance of this task"
                            }
                        },
                        "required": ["content", "status", "priority"]
                    }
                }
            },
            "required": ["todos"]
        }),
    }
}

fn parse_todo_entry(value: &Value) -> Result<TodoEntry, ToolFailure> {
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Each todo requires 'content' (string)"))?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Each todo requires 'status' (string)"))?;
    let priority = value
        .get("priority")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Each todo requires 'priority' (string)"))?;

    if !matches!(status, "pending" | "in_progress" | "completed") {
        return Err(ToolFailure::invalid_input(format!(
            "Invalid status '{}': must be pending|in_progress|completed",
            status
        )));
    }
    if !matches!(priority, "high" | "medium" | "low") {
        return Err(ToolFailure::invalid_input(format!(
            "Invalid priority '{}': must be high|medium|low",
            priority
        )));
    }

    Ok(TodoEntry {
        id: uuid_v4_simple(),
        content: content.to_string(),
        status: status.to_string(),
        priority: priority.to_string(),
    })
}

// 轻量级 UUID 生成，避免引入 uuid crate 依赖。
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ts = now.as_nanos();
    format!("{:016x}{:016x}", ts, ts.wrapping_mul(2654435761))
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let todos_raw = input
        .get("todos")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: todos (array)"))?;

    if todos_raw.is_empty() {
        // 空列表：清空当前会话的待办。
        global_registry().replace_all(conversation_id, Vec::new());
        emit_todo_updated(app, conversation_id);
        return Ok(ToolOutcome::text("Todo list cleared."));
    }

    let mut todos = Vec::with_capacity(todos_raw.len());
    for (idx, item) in todos_raw.iter().enumerate() {
        let entry = parse_todo_entry(item).map_err(|e| {
            ToolFailure::invalid_input(format!("todos[{}] invalid: {}", idx, e.message))
        })?;
        todos.push(entry);
    }

    // 校验：最多一个 in_progress。
    let in_progress_count = todos
        .iter()
        .filter(|t| t.status == "in_progress")
        .count();
    if in_progress_count > 1 {
        return Err(ToolFailure::invalid_input(format!(
            "At most one todo can be 'in_progress' at a time, got {}",
            in_progress_count
        )));
    }

    let saved = global_registry().replace_all(conversation_id, todos.clone());
    emit_todo_updated(app, conversation_id);
    let rendered = render_todo_list(&saved);
    Ok(ToolOutcome::text(rendered))
}

/// 工具执行完成后通知前端刷新待办列表 UI。
/// 事件 payload 携带 conversation_id，前端按当前会话过滤接收。
fn emit_todo_updated(app: &AppHandle, conversation_id: Option<&str>) {
    // 取主窗口 emit；多窗口场景按需扩展。
    if let Some(window) = app.get_webview_window("main") {
        let payload = serde_json::json!({
            "conversationId": conversation_id,
        });
        let _ = window.emit("todo-updated", payload);
    }
}

fn render_todo_list(todos: &[TodoEntry]) -> String {
    if todos.is_empty() {
        return "Todo list is empty.".to_string();
    }
    let mut out = String::from("Todos updated:\n");
    for (idx, todo) in todos.iter().enumerate() {
        let marker = match todo.status.as_str() {
            "completed" => "[x]",
            "in_progress" => "[>]",
            _ => "[ ]",
        };
        let prio = match todo.priority.as_str() {
            "high" => "HIGH",
            "low" => "LOW",
            _ => "MED",
        };
        out.push_str(&format!(
            "  {} #{} ({}) {}\n",
            marker, idx + 1, prio, todo.content
        ));
    }
    let done = todos.iter().filter(|t| t.status == "completed").count();
    out.push_str(&format!(
        "\nProgress: {}/{} completed",
        done,
        todos.len()
    ));
    out
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
