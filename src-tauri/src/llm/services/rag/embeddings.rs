use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use url::Url;

const EMBEDDING_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingDatum {
    index: usize,
    embedding: Vec<f32>,
}

pub fn embedding_to_blob(values: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(values.len() * std::mem::size_of::<f32>());
    for value in values {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

pub fn load_embedding_config(app: &AppHandle) -> Result<EmbeddingConfig, String> {
    let settings = crate::command::settings::get_settings(app.clone())?;
    let model = settings.rag.embedding_model.trim().to_string();
    if model.is_empty() {
        return Err("RAG embedding model is not configured".to_string());
    }

    let protocol = settings.active_provider_protocol();
    if protocol == "anthropic" {
        return Err("RAG embeddings require an OpenAI-compatible provider profile".to_string());
    }

    let profile = settings.active_provider_profile();
    let api_key = profile.api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("RAG embedding provider API key is not configured".to_string());
    }

    let base_url = profile.base_url.trim();
    if base_url.is_empty() {
        return Err("RAG embedding provider base_url is not configured".to_string());
    }

    Ok(EmbeddingConfig {
        model,
        endpoint: normalize_embedding_endpoint(base_url)?,
        api_key,
    })
}

pub async fn embed_texts(
    config: &EmbeddingConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    let client = Client::new();
    let mut embeddings = Vec::with_capacity(texts.len());
    for batch in texts.chunks(EMBEDDING_BATCH_SIZE) {
        let request = EmbeddingRequest {
            model: &config.model,
            input: batch,
        };
        let response = client
            .post(&config.endpoint)
            .bearer_auth(&config.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Embedding request failed: {}", e))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read embedding response: {}", e))?;
        if !status.is_success() {
            return Err(format!(
                "Embedding request failed with HTTP {}: {}",
                status,
                truncate_error_body(&body)
            ));
        }

        let mut parsed = serde_json::from_str::<EmbeddingResponse>(&body)
            .map_err(|e| format!("Failed to parse embedding response: {}", e))?;
        parsed.data.sort_by_key(|item| item.index);
        if parsed.data.len() != batch.len() {
            return Err(format!(
                "Embedding response count mismatch: expected {}, got {}",
                batch.len(),
                parsed.data.len()
            ));
        }
        for item in parsed.data {
            if item.embedding.is_empty() {
                return Err("Embedding response contained an empty vector".to_string());
            }
            embeddings.push(item.embedding);
        }
    }

    Ok(embeddings)
}

fn truncate_error_body(body: &str) -> String {
    let mut chars = body.chars();
    let preview = chars.by_ref().take(500).collect::<String>();
    if chars.next().is_some() {
        format!("{}...", preview)
    } else {
        preview
    }
}

fn normalize_embedding_endpoint(raw_base_url: &str) -> Result<String, String> {
    let mut url = Url::parse(raw_base_url).map_err(|e| {
        format!(
            "Invalid embedding provider base_url '{}': {}",
            raw_base_url, e
        )
    })?;
    let path = url.path().trim_end_matches('/').to_string();
    let next_path = if path.ends_with("/v1/embeddings") || path.ends_with("/embeddings") {
        path
    } else if path.ends_with("/v1/chat/completions") {
        path.trim_end_matches("/chat/completions").to_string() + "/embeddings"
    } else if path.ends_with("/chat/completions") {
        path.trim_end_matches("/chat/completions").to_string() + "/embeddings"
    } else if path.ends_with("/v1/responses") {
        path.trim_end_matches("/responses").to_string() + "/embeddings"
    } else if path.ends_with("/responses") {
        path.trim_end_matches("/responses").to_string() + "/embeddings"
    } else if path.ends_with("/v1") {
        format!("{}/embeddings", path)
    } else {
        format!("{}/v1/embeddings", path)
    };

    url.set_path(&next_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}
