use serde_json::{json, Value};

use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        tool,
        execute_spawn_agent_boxed,
        false,
        None,
        ToolDisclosure::Core,
    )]
}

pub fn tool() -> Tool {
    Tool {
        name: "spawn_agent".into(),
        description: r#"Spawns an autonomous subagent with a dedicated role and task asynchronously in the background.
The subagent runs in its own cognitive loop, is registered into the multi-agent registry, connects to the collaborative message board, and returns immediately so you can continue working. Use `wait_agent` to await its final report or `send_message` to guide it."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "role": {
                    "type": "string",
                    "description": "The functional role of the agent, e.g. 'researcher', 'reviewer', 'architect', 'debugger'"
                },
                "nickname": {
                    "type": "string",
                    "description": "Short mnemonic name for the subagent, e.g. 'CodeExplorer' or 'SyntaxAuditor'"
                },
                "task": {
                    "type": "string",
                    "description": "Detailed task instructions, files to explore, or questions to answer"
                }
            },
            "required": ["task"]
        }),
    }
}

fn execute_spawn_agent_boxed(
    core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let parent_id = conversation_id.unwrap_or_else(|| "default".to_string());
        let task = input
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: task"))?;

        let role = input
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("researcher");

        let nickname = input
            .get("nickname")
            .and_then(Value::as_str)
            .unwrap_or("Subagent");

        let spawned = crate::agent::orchestration::subagent::spawn_async(
            &core,
            &parent_id,
            task,
            Some(role),
            Some(nickname),
        )
        .await
        .map_err(ToolFailure::new)?;

        Ok(ToolOutcome::json(spawned))
    })
}
