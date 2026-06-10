use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "config_tool".into(),
        description: "Read a redacted Nova runtime configuration summary. This tool is read-only and never returns secrets or writes settings.".into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get_runtime_config_summary"],
                    "description": "Only supported operation."
                }
            },
            "required": ["action"]
        }),
    }
}

fn base_url_summary(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(url) = url::Url::parse(trimmed) else {
        return Some("<custom>".to_string());
    };

    let host = url.host_str()?;
    let mut summary = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        summary.push(':');
        summary.push_str(&port.to_string());
    }
    Some(summary)
}

async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let action = match input.get("action").and_then(Value::as_str) {
        Some(value) => value.trim(),
        None => return Err(ToolFailure::invalid_input("Missing 'action' argument")),
    };

    if action != "get_runtime_config_summary" {
        return Err(ToolFailure::invalid_input(
            "config_tool only supports get_runtime_config_summary",
        ));
    }

    let settings = match crate::command::settings::get_settings(app.clone()) {
        Ok(settings) => settings,
        Err(error) => return Err(ToolFailure::new(error)),
    };
    let active_provider = settings.active_provider_key();
    let active_profile = settings.active_provider_profile();
    let active_protocol = settings.active_provider_api_format();
    let active_custom_model_count = settings
        .custom_models
        .get(&active_provider)
        .map(Vec::len)
        .unwrap_or(0);

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "action": "get_runtime_config_summary",
        "provider": {
            "active": active_provider,
            "protocol": active_protocol,
            "displayName": active_profile.display_name,
            "model": active_profile.model,
            "hasApiKey": !active_profile.api_key.trim().is_empty(),
            "baseUrl": base_url_summary(&active_profile.base_url),
            "profileCount": settings.provider_profiles.len(),
            "activeCustomModelCount": active_custom_model_count
        },
        "skills": {
            "disabledCount": settings.disabled_skills.len()
        },
        "hooks": {
            "envKeyCount": settings.hook_env.len()
        },
        "rag": {
            "embeddingModelConfigured": !settings.rag.embedding_model.trim().is_empty(),
            "storage": "sqlite",
            "textRetrieval": "sqlite_fts5",
            "vectorRetrieval": "sqlite_vec"
        },
        "ui": {
            "language": settings.ui_language,
            "theme": settings.ui_theme,
            "appLogEnabled": settings.enable_app_log
        },
        "redacted": true
    })))
}
