use crate::llm::tools::{app_tool, AppExecuteFuture, ToolDisclosure, ToolOutcome, ToolRegistration};
use crate::llm::services::plan_files;
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

// 注册 exit_plan_mode，声明它是无权限要求的同步状态切换工具。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None, ToolDisclosure::Core)
}

// 返回暴露给模型的工具元数据，告诉模型这个工具用于退出 plan 模式。
pub fn tool() -> Tool {
    Tool {
        name: "exit_plan_mode".into(),
        description: "Exit plan mode after the planning phase is complete and resume normal implementation work. You MUST pass the full final plan text via `plan`; it is saved as the conversation plan and shown to the user as a structured panel.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "REQUIRED. The full final plan in Markdown: a concise title, context/background, goal, numbered implementation steps, and verification notes."
                },
                "summary": {
                    "type": "string",
                    "description": "Optional one-line summary of the agreed plan"
                }
            },
            "required": ["plan"]
        }),
    }
}

// 读取 plan 全文并写入应用数据 plans 目录（每会话仅一份，覆盖旧版），
// 再返回 plan_mode_change payload 给前端切换模式并刷新 Plan 面板。
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
        .ok_or_else(|| "exit_plan_mode 需要 plan 参数（完整计划文本）".to_string())?;

    let summary = input
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let saved = plan_files::save_conversation_plan(app, conversation_id, &plan)?;
    emit_plan_updated(app, conversation_id, &saved.content, saved.updated_at);

    Ok(json!({
        "type": "plan_mode_change",
        "mode": "default",
        "summary": summary,
        "plan": saved.content,
        "planUpdatedAt": saved.updated_at,
        "message": "Exited plan mode. The plan was saved and shown to the user. You may now implement the approved plan."
    })
    .to_string())
}

// 通知前端刷新 Plan 面板（与 TodoWrite 的 todo-updated 事件模式一致）。
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
