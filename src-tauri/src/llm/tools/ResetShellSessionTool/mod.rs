use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
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
            Err(error) => {
                return json!({
                    "ok": false,
                    "error": error
                })
                .to_string();
            }
        };

        match crate::llm::services::shell_sessions::reset_session(
            conversation_id.as_deref(),
            Some(&workspace_root),
        )
        .await
        {
            Ok(()) => json!({
                "ok": true,
                "message": "Shell session reset."
            })
            .to_string(),
            Err(error) => json!({
                "ok": false,
                "error": error
            })
            .to_string(),
        }
    })
}
