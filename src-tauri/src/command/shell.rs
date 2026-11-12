//! 桌面薄壳 IPC：转发至 nova daemon（与内核 dispatch / api/shell 一一对应）。

use nova_protocol::misc::BackgroundOutput;

#[tauri::command]
pub async fn list_background_jobs(conversation_id: Option<String>) -> Result<Vec<BackgroundOutput>, String> {
    crate::daemon::invoke("list_background_jobs", serde_json::json!({"conversationId": conversation_id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn read_background_job_output(conversation_id: Option<String>, id: String, tail_lines: Option<usize>) -> Result<BackgroundOutput, String> {
    crate::daemon::invoke("read_background_job_output", serde_json::json!({"conversationId": conversation_id, "id": id, "tailLines": tail_lines})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn kill_background_job(conversation_id: Option<String>, id: String) -> Result<BackgroundOutput, String> {
    crate::daemon::invoke("kill_background_job", serde_json::json!({"conversationId": conversation_id, "id": id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
