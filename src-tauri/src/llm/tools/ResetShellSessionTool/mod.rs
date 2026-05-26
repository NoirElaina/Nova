use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "reset_shell_session".into(),
        description: "Reset the current conversation's persistent shell session. Use this when the shell environment is polluted or background processes should be stopped before continuing.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let workspace_root = match crate::command::workspace::workspace_root_string_for_conversation(
            &app,
            conversation_id.as_deref(),
        ) {
            Ok(root) => root,
            Err(error) => return Err(ToolFailure::new(error)),
        };

        match crate::llm::services::shell_sessions::reset_session(
            conversation_id.as_deref(),
            Some(&workspace_root),
        )
        .await
        {
            Ok(()) => Ok(ToolOutcome::json(json!({
                "ok": true,
                "message": "Shell session reset."
            }))),
            Err(error) => Err(ToolFailure::new(error)),
        }
    })
}
