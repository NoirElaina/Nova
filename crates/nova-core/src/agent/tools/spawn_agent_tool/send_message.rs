use serde_json::{json, Value};

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        tool,
        execute_send_message_boxed,
        false,
        None,
        ToolDisclosure::Core,
    )]
}

pub fn tool() -> Tool {
    Tool {
        name: "send_message".into(),
        description: r#"Sends an asynchronous message or guidance directive to an active subagent's mailbox, and broadcasts to the team channel."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Target subagent ID or nickname (e.g. 'ResearchSubagent' or full ID)"
                },
                "message": {
                    "type": "string",
                    "description": "The message, guidance, or feedback to deliver"
                }
            },
            "required": ["target", "message"]
        }),
    }
}

fn execute_send_message_boxed(
    _core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let parent_id = conversation_id.unwrap_or_else(|| "default".to_string());
        let target = input
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: target"))?;
        let message = input
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: message"))?;

        crate::agent::orchestration::registry::global_agent_registry()
            .send_to_mailbox(&parent_id, target, message)
            .await
            .map_err(ToolFailure::new)?;

        // 同步在协同看板 #general 频道发帖通知
        let _ = crate::agent::orchestration::global_message_board().post_message(
            &parent_id,
            "general",
            None,
            "coordinator",
            "Coordinator",
            "lead",
            &format!("@{} {}", target, message),
        );

        Ok(ToolOutcome::json(json!({
            "status": "delivered",
            "target": target,
            "message": format!("Message delivered to {}'s mailbox", target)
        })))
    })
}
