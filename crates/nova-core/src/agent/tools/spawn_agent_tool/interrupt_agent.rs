use serde_json::{json, Value};

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        tool,
        execute_interrupt_agent_boxed,
        false,
        None,
        ToolDisclosure::Core,
    )]
}

pub fn tool() -> Tool {
    Tool {
        name: "interrupt_agent".into(),
        description: r#"Interrupts and halts an actively running subagent by target ID or nickname."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Subagent ID or nickname to interrupt (e.g. 'ResearchSubagent' or full agent ID)"
                }
            },
            "required": ["target"]
        }),
    }
}

fn execute_interrupt_agent_boxed(
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

        crate::agent::orchestration::registry::global_agent_registry()
            .interrupt(&parent_id, target)
            .map_err(ToolFailure::new)?;

        // 同步通知看板
        let _ = crate::agent::orchestration::global_message_board().post_message(
            &parent_id,
            "general",
            None,
            "coordinator",
            "Coordinator",
            "lead",
            &format!("Agent '{}' was interrupted by coordinator", target),
        );

        Ok(ToolOutcome::json(json!({
            "status": "interrupted",
            "target": target,
            "message": format!("Subagent '{}' has been successfully interrupted", target)
        })))
    })
}
