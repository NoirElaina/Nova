use crate::llm::services::plan_files;
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolDisclosure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

// 注册 write_plan：唯一的计划工具，直接写入计划，无进入/退出模式概念。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None, ToolDisclosure::Core)
}

// 返回暴露给模型的工具元数据：直接把完整计划写入会话。
pub fn tool() -> Tool {
    Tool {
        name: "write_plan".into(),
        description: "Write or update the conversation plan directly. Pass the full plan text via `plan`; it is saved as the conversation plan and shown to the user as a structured panel. Use it when the user asks for a plan or before starting a complex multi-step task. There is no separate plan mode — keep working normally after writing the plan.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "REQUIRED. The full plan in Markdown: a concise title, context/background, goal, numbered implementation steps, and verification notes."
                },
                "summary": {
                    "type": "string",
                    "description": "Optional one-line summary of the plan"
                }
            },
            "required": ["plan"]
        }),
    }
}

// 把计划全文写入应用数据 plans 目录（每会话仅一份，覆盖旧版），
// 并通知前端刷新计划面板。
fn execute_local(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<String, String> {
    let plan = input
        .get("plan")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "write_plan 需要 plan 参数（完整计划文本）".to_string())?;

    let summary = input
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let saved = plan_files::save_conversation_plan(app, conversation_id, &plan)?;
    emit_plan_updated(app, conversation_id, &saved.content, saved.updated_at);

    Ok(json!({
        "type": "plan_saved",
        "summary": summary,
        "planUpdatedAt": saved.updated_at,
        "message": "The plan was saved and shown to the user. Continue working normally."
    })
    .to_string())
}

// 通知前端刷新计划面板（与 TodoWrite 的 todo-updated 事件模式一致）。
fn emit_plan_updated(
    app: &AppHandle,
    conversation_id: Option<&str>,
    content: &str,
    updated_at: i64,
) {
    if let Some(window) = app.get_webview_window("main") {
        let payload = serde_json::json!({
            "conversationId": conversation_id,
            "content": content,
            "updatedAt": updated_at,
        });
        let _ = window.emit("plan-updated", payload);
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        match execute_local(&app, conversation_id.as_deref(), input) {
            Ok(text) => Ok(ToolOutcome::text(text)),
            Err(message) => Ok(ToolOutcome::text(
                json!({
                    "type": "error",
                    "message": message
                })
                .to_string(),
            )),
        }
    })
}
