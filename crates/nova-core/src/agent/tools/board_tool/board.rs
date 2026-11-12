use serde_json::{json, Value};

use crate::agent::orchestration::board::global_message_board;
use crate::agent::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::core::Core;
use crate::provider::types::Tool;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![
        app_tool(
            board_post_tool,
            execute_post_boxed,
            false,
            None,
            ToolDisclosure::Core,
        ),
        app_tool(
            board_read_tool,
            execute_read_boxed,
            true,
            None,
            ToolDisclosure::Core,
        ),
        app_tool(
            board_list_channels_tool,
            execute_list_channels_boxed,
            true,
            None,
            ToolDisclosure::Core,
        ),
    ]
}

pub fn board_post_tool() -> Tool {
    Tool {
        name: "board_post".into(),
        description: r#"Posts a message, finding, or evidence to the shared collaborative message board.
Built-in core tool (NOT an MCP tool; NO LoadTool needed). Other collaborating agents and the user see this in real-time across channels (#general, #research, #evidence)."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "description": "Channel name to post to, e.g. 'general', 'research', or 'evidence'"
                },
                "content": {
                    "type": "string",
                    "description": "The message body, code references, or evidence to post"
                },
                "thread_id": {
                    "type": "string",
                    "description": "Optional thread ID to reply to an existing discussion thread"
                }
            },
            "required": ["channel", "content"]
        }),
    }
}

pub fn board_read_tool() -> Tool {
    Tool {
        name: "board_read".into(),
        description: r#"Reads recent messages from a channel or topic thread on the collaborative message board.
Built-in core tool (NOT an MCP tool; NO LoadTool needed)."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "description": "Channel name to read, e.g. 'general', 'research', 'evidence'"
                },
                "thread_id": {
                    "type": "string",
                    "description": "Optional thread ID to read all replies in that thread"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of messages to return (default 20)"
                }
            }
        }),
    }
}

pub fn board_list_channels_tool() -> Tool {
    Tool {
        name: "board_list_channels".into(),
        description: r#"Lists all active channels on the collaborative message board along with their descriptions and message counts.
Built-in core tool (NOT an MCP tool; NO LoadTool needed)."#.into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn execute_post_boxed(
    core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let conv_id = conversation_id.unwrap_or_else(|| "default".to_string());
        let root_id = crate::agent::orchestration::subagent::parent_conversation_id(&conv_id).to_string();

        let channel = input
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: channel"))?;

        let content = input
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: content"))?;

        let thread_id = input
            .get("thread_id")
            .and_then(Value::as_str)
            .map(ToString::to_string);

        let (author_id, author_nickname, author_role) = if crate::agent::orchestration::subagent::is_subagent_conversation(Some(&conv_id)) {
            (conv_id.clone(), "Subagent".to_string(), "subagent".to_string())
        } else {
            ("root".to_string(), "LeadAgent".to_string(), "lead".to_string())
        };

        let post = global_message_board().post_message(
            &root_id,
            channel,
            thread_id,
            &author_id,
            &author_nickname,
            &author_role,
            content,
        ).map_err(ToolFailure::new)?;

        core.emit(crate::events::NovaEvent::BoardPostCreated {
            conversation_id: root_id,
            post: serde_json::to_value(&post).unwrap_or_default(),
        });

        Ok(ToolOutcome::json(json!({
            "ok": true,
            "post": post
        })))
    })
}

fn execute_read_boxed(
    _core: Core,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let conv_id = conversation_id.unwrap_or_else(|| "default".to_string());
        let root_id = crate::agent::orchestration::subagent::parent_conversation_id(&conv_id);

        let channel = input.get("channel").and_then(Value::as_str);
        let thread_id = input.get("thread_id").and_then(Value::as_str);
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;

        let posts = global_message_board().list_posts(root_id, channel, thread_id, limit);

        Ok(ToolOutcome::json(json!({
            "ok": true,
            "count": posts.len(),
            "posts": posts
        })))
    })
}

fn execute_list_channels_boxed(
    _core: Core,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let conv_id = conversation_id.unwrap_or_else(|| "default".to_string());
        let root_id = crate::agent::orchestration::subagent::parent_conversation_id(&conv_id);

        let channels = global_message_board().list_channels(root_id);

        Ok(ToolOutcome::json(json!({
            "ok": true,
            "channels": channels
        })))
    })
}
