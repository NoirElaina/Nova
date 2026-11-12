
use crate::agent::utils::error_event::report_backend_result;

pub use nova_protocol::misc::FetchedModel;

pub async fn fetch_available_models(
    core: &crate::core::Core,
    base_url: String,
    api_key: String,
    is_full_url: Option<bool>,
    models_url_override: Option<String>,
    api_format: Option<String>,
) -> Result<Vec<FetchedModel>, String> {
    let result = crate::provider::model_fetch::fetch_models(
        &base_url,
        &api_key,
        is_full_url.unwrap_or(false),
        models_url_override.as_deref(),
        api_format.as_deref(),
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
        core,
        "command.model_fetch.fetch_available_models",
        result,
        None,
    )
}
