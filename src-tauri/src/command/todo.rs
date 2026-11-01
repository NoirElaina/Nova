//! 桌面薄壳 IPC：转发至 nova daemon（方法名 todo.* 与内核分发器一一对应）。

pub use nova_protocol::misc::TodoEntry;

#[tauri::command]
pub async fn list_todos(conversation_id: Option<String>) -> Result<Vec<TodoEntry>, String> {
    crate::daemon::invoke("list_todos", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
