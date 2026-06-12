use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: u64,
    pub title: String,
    pub description: Option<String>,
    pub status: GoalStatus,
    pub token_budget: Option<u64>,
    pub token_used: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    Active,
    Complete,
    Blocked,
}

// 每个会话只有一个目标，用 conversation_id 作为 key
static STORE: OnceLock<RwLock<HashMap<String, Goal>>> = OnceLock::new();

fn get_store() -> &'static RwLock<HashMap<String, Goal>> {
    STORE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn conversation_key(conversation_id: Option<&str>) -> String {
    conversation_id.unwrap_or("default").to_string()
}

fn now_str() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

pub fn create_goal_registration() -> ToolRegistration {
    app_tool(create_goal_tool, create_goal_execute, false, None)
}

fn create_goal_tool() -> Tool {
    Tool {
        name: "create_goal".into(),
        description: "Create a goal for the current session. Only one active goal per conversation. Optionally set a token budget to track resource usage. If a goal already exists, it will be replaced.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Goal title" },
                "description": { "type": "string", "description": "Optional detailed description" },
                "token_budget": { "type": "integer", "description": "Optional token budget limit" }
            },
            "required": ["title"]
        }),
    }
}

fn create_goal_execute(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| ToolFailure::invalid_input("Missing 'title'"))?
            .to_string();

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let token_budget = input.get("token_budget").and_then(|v| v.as_u64());

        let store = get_store();
        let mut store = store.write().await;
        let key = conversation_key(conversation_id.as_deref());

        let now = now_str();

        // 如果已有目标，复用 id
        let id = store.get(&key).map(|g| g.id).unwrap_or(1);

        let goal = Goal {
            id,
            title,
            description,
            status: GoalStatus::Active,
            token_budget,
            token_used: 0,
            created_at: now.clone(),
            updated_at: now,
        };

        store.insert(key, goal.clone());

        Ok(ToolOutcome::json(json!({ "ok": true, "goal": goal })))
    })
}

pub fn update_goal_registration() -> ToolRegistration {
    app_tool(update_goal_tool, update_goal_execute, false, None)
}

fn update_goal_tool() -> Tool {
    Tool {
        name: "update_goal".into(),
        description: "Update the current goal's status to 'complete' or 'blocked', or update token usage.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["complete", "blocked"], "description": "New status" },
                "token_used": { "type": "integer", "description": "Update token usage count" }
            }
        }),
    }
}

fn update_goal_execute(
    _app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let store = get_store();
        let mut store = store.write().await;
        let key = conversation_key(conversation_id.as_deref());

        let goal = store
            .get_mut(&key)
            .ok_or_else(|| ToolFailure::new("No active goal. Use create_goal first."))?;

        if let Some(status_str) = input.get("status").and_then(|v| v.as_str()) {
            goal.status = match status_str.to_lowercase().as_str() {
                "complete" => GoalStatus::Complete,
                "blocked" => GoalStatus::Blocked,
                _ => GoalStatus::Active,
            };
        }
        if let Some(tokens) = input.get("token_used").and_then(|v| v.as_u64()) {
            goal.token_used = tokens;
        }
        goal.updated_at = now_str();

        Ok(ToolOutcome::json(json!({ "ok": true, "goal": goal.clone() })))
    })
}

pub fn get_goal_registration() -> ToolRegistration {
    app_tool(get_goal_tool, get_goal_execute, true, None)
}

fn get_goal_tool() -> Tool {
    Tool {
        name: "get_goal".into(),
        description: "Get the current goal for this conversation, including status and token consumption.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn get_goal_execute(
    _app: AppHandle,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let store = get_store();
        let store = store.read().await;
        let key = conversation_key(conversation_id.as_deref());

        match store.get(&key) {
            Some(goal) => Ok(ToolOutcome::json(json!({ "ok": true, "goal": goal }))),
            None => Ok(ToolOutcome::json(json!({ "ok": true, "goal": null, "message": "No active goal" }))),
        }
    })
}
