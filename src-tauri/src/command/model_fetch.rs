use serde::Serialize;
use tauri::AppHandle;

use crate::llm::utils::error_event::report_backend_result;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub owned_by: Option<String>,
}

#[tauri::command]
pub async fn fetch_available_models(
    app: AppHandle,
    base_url: String,
    api_key: String,
    is_full_url: Option<bool>,
    models_url_override: Option<String>,
) -> Result<Vec<FetchedModel>, String> {
    let result = crate::llm::services::model_fetch::fetch_models(
        &base_url,
        &api_key,
        is_full_url.unwrap_or(false),
        models_url_override.as_deref(),
    )
    .await
    .map(|models| {
        models
            .into_iter()
            .map(|m| FetchedModel {
                id: m.id,
                owned_by: m.owned_by,
            })
            .collect()
    });

    report_backend_result(
        &app,
        "command.model_fetch.fetch_available_models",
        result,
        None,
    )
}
