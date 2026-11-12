use serde_json::{json, Value};

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        tool,
        execute_list_agents_boxed,
        false,
        None,
        ToolDisclosure::Core,
    )]
}

pub fn tool() -> Tool {
    Tool {
        name: "list_agents".into(),
        description: r#"Lists all spawned subagents in the current conversation, along with their roles, nicknames, status (running, completed, errored, interrupted), and result previews."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

fn execute_list_agents_boxed(
    _core: Core,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let parent_id = conversation_id.unwrap_or_else(|| "default".to_string());
        let agents = crate::agent::orchestration::registry::global_agent_registry()
            .list_summaries(&parent_id);

        Ok(ToolOutcome::json(json!({
            "agents": agents,
            "total": agents.len()
        })))
    })
}
