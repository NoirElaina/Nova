use crate::agent::orchestration::board::{global_message_board, BoardChannel, BoardPost};
use crate::agent::orchestration::graph::{global_graph_store, AgentNode};
use crate::agent::orchestration::subagent::parent_conversation_id;
use crate::core::Core;

pub async fn get_agent_graph(
    _core: &Core,
    conversation_id: String,
) -> Result<Vec<AgentNode>, String> {
    let root_id = parent_conversation_id(&conversation_id);
    Ok(global_graph_store().get_all_nodes(root_id))
}

pub async fn get_board_channels(
    _core: &Core,
    conversation_id: String,
) -> Result<Vec<BoardChannel>, String> {
    let root_id = parent_conversation_id(&conversation_id);
    Ok(global_message_board().list_channels(root_id))
}

pub async fn get_board_posts(
    _core: &Core,
    conversation_id: String,
    channel: Option<String>,
    thread_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<BoardPost>, String> {
    let root_id = parent_conversation_id(&conversation_id);
    Ok(global_message_board().list_posts(
        root_id,
        channel.as_deref(),
        thread_id.as_deref(),
        limit.unwrap_or(50),
    ))
}

#[derive(Debug, Clone)]
pub struct PostBoardMessageInput {
    pub conversation_id: String,
    pub channel: String,
    pub content: String,
    pub thread_id: Option<String>,
    pub author_nickname: Option<String>,
    pub author_role: Option<String>,
    pub idempotency_key: Option<String>,
}

pub async fn post_board_message(
    core: &Core,
    input: PostBoardMessageInput,
) -> Result<BoardPost, crate::NovaCoreError> {
    if input.content.trim().is_empty() {
        return Err(crate::NovaCoreError::InvalidInput(
            "Post content cannot be empty".to_string(),
        ));
    }

    let idempotency_key = input.idempotency_key;
    core.run_idempotent("post_board_message", idempotency_key.as_deref(), || async move {
        let root_id = parent_conversation_id(&input.conversation_id).to_string();
        let nickname = input.author_nickname.unwrap_or_else(|| "User".to_string());
        let role = input.author_role.unwrap_or_else(|| "user".to_string());

        let post = global_message_board()
            .post_message(
                &root_id,
                &input.channel,
                input.thread_id,
                "user",
                &nickname,
                &role,
                &input.content,
            )
            .map_err(crate::NovaCoreError::Session)?;

        core.emit(crate::events::NovaEvent::BoardPostCreated {
            conversation_id: root_id,
            post: serde_json::to_value(&post).unwrap_or_default(),
        });

        Ok(post)
    })
    .await
}

pub async fn interrupt_agent(
    _core: &Core,
    conversation_id: String,
    agent_id: String,
) -> Result<bool, String> {
    let parent_id = parent_conversation_id(&conversation_id);
    crate::agent::orchestration::registry::global_agent_registry()
        .interrupt(parent_id, &agent_id)
        .map(|_| true)
}

pub async fn send_agent_message(
    _core: &Core,
    conversation_id: String,
    agent_id: String,
    directive: String,
) -> Result<bool, String> {
    let parent_id = parent_conversation_id(&conversation_id);
    crate::agent::orchestration::registry::global_agent_registry()
        .send_to_mailbox(parent_id, &agent_id, &directive)
        .await
        .map(|_| true)
}
