//! 桌面薄壳 IPC：转发至 nova daemon（方法名 cron.* 与内核分发器一一对应）。

use nova_protocol::misc::CronJob;

#[tauri::command]
pub async fn list_scheduled_tasks() -> Result<Vec<CronJob>, String> {
    crate::daemon::invoke("list_scheduled_tasks", serde_json::json!({})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn create_scheduled_task(cron: String, prompt: String, recurring: Option<bool>, durable: Option<bool>) -> Result<CronJob, String> {
    crate::daemon::invoke("create_scheduled_task", serde_json::json!({"cron": cron, "prompt": prompt, "recurring": recurring, "durable": durable})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn delete_scheduled_task(id: String) -> Result<bool, String> {
    crate::daemon::invoke("delete_scheduled_task", serde_json::json!({"id": id})).await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
