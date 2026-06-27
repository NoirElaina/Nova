use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把记忆操作包装成统一 future。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 remember_global_memory 工具注册信息（写类工具）。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 模型可见的 memory 工具元数据。
// schema 对齐 hermes MEMORY_SCHEMA 的简化版（去掉 target/batch，单文件 MEMORY.md）：
// - action: add | replace | remove
// - content: 新内容（add/replace 必填）
// - old_text: 子串匹配（replace/remove 必填）
pub fn tool() -> Tool {
    Tool {
        name: "memory".into(),
        description: "Save durable facts to persistent memory that survive across sessions. Memory is injected into every future turn, so keep entries compact and high-signal.\n\nWHEN: save proactively when the user states a preference, correction, or personal detail, or you learn a stable fact about their environment, conventions, or workflow. Priority: user preferences & corrections > environment facts > procedures. The best memory stops the user repeating themselves.\n\nSKIP: trivial/obvious info, easily re-discovered facts, raw data dumps, task progress, completed-work logs, temporary TODO state.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "replace", "remove"],
                    "description": "The action to perform."
                },
                "content": {
                    "type": "string",
                    "description": "The entry content. Required for 'add' and 'replace'."
                },
                "old_text": {
                    "type": "string",
                    "description": "REQUIRED for 'replace' and 'remove': a short unique substring identifying the existing entry to modify. Omit for 'add'."
                }
            },
            "required": ["action"]
        }),
    }
}

// 执行 memory 工具调用，按 action 分发到 memory_add/replace/remove。
async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let action = match input.get("action").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return Err(ToolFailure::invalid_input("Missing 'action' argument")),
    };

    let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let old_text = input.get("old_text").and_then(|v| v.as_str()).unwrap_or("");

    let result = match action {
        "add" => {
            if content.trim().is_empty() {
                return Err(ToolFailure::invalid_input("'add' requires non-empty 'content'"));
            }
            crate::llm::services::memory_dir::memory_add(app, content).await
        }
        "replace" => {
            if old_text.trim().is_empty() {
                return Err(ToolFailure::invalid_input(
                    "'replace' requires 'old_text' (a short unique substring of the entry to modify)",
                ));
            }
            if content.trim().is_empty() {
                return Err(ToolFailure::invalid_input(
                    "'replace' requires non-empty 'content' (use 'remove' to delete)",
                ));
            }
            crate::llm::services::memory_dir::memory_replace(app, old_text, content).await
        }
        "remove" => {
            if old_text.trim().is_empty() {
                return Err(ToolFailure::invalid_input(
                    "'remove' requires 'old_text' (a short unique substring of the entry to remove)",
                ));
            }
            crate::llm::services::memory_dir::memory_remove(app, old_text).await
        }
        other => {
            return Err(ToolFailure::invalid_input(&format!(
                "Unknown action '{}'. Use: add, replace, or remove.",
                other
            )));
        }
    };

    match result {
        Ok(()) => Ok(ToolOutcome::json(json!({
            "ok": true,
            "action": action,
            "done": true,
            "note": "Write saved. This update is complete — do not repeat it."
        }))),
        Err(e) => Err(ToolFailure::new(e)),
    }
}
