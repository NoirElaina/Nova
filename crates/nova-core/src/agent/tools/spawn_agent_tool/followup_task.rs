use serde_json::{json, Value};

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        tool,
        execute_followup_task_boxed,
        false,
        None,
        ToolDisclosure::Core,
    )]
}

pub fn tool() -> Tool {
    Tool {
        name: "followup_task".into(),
        description: r#"Assigns a follow-up task or new objective to an active subagent, delivering it to the agent's mailbox and notifying the team channel."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Target subagent ID or nickname (e.g. 'ResearchSubagent' or full subagent ID)"
                },
                "task": {
                    "type": "string",
                    "description": "Follow-up instructions or new objective to assign"
                }
            },
            "required": ["target", "task"]
        }),
    }
}

fn execute_followup_task_boxed(
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
        let task = input
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: task"))?;

        let formatted_message = format!("[Follow-up Task Directive]: {}", task);

        crate::agent::orchestration::registry::global_agent_registry()
            .send_to_mailbox(&parent_id, target, &formatted_message)
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
            &format!("@{} [Follow-up Task]: {}", target, task),
        );

        Ok(ToolOutcome::json(json!({
            "status": "assigned",
            "target": target,
            "task": task,
            "message": format!("Follow-up task assigned to {}'s mailbox", target)
        })))
    })
}
