//! 模型列表获取服务
//!
//! 通过 OpenAI 兼容的 GET /v1/models 端点获取供应商可用模型列表。
//! 主要面向第三方聚合站（硅基流动、OpenRouter 等），以及把 Anthropic
//! 协议挂在兼容子路径上的官方供应商（DeepSeek、Kimi、智谱 GLM 等）。
//! Anthropic 格式时使用 x-api-key 认证（官方 /v1/models 不认 Bearer）。

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub owned_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<ModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
    /// Anthropic 官方列表返回 display_name 而非标准 id，作兜底。
    display_name: Option<String>,
    owned_by: Option<String>,
}

const FETCH_TIMEOUT_SECS: u64 = 15;
const ERROR_BODY_MAX_CHARS: usize = 512;

const KNOWN_COMPAT_SUFFIXES: &[&str] = &[
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/step_plan",
    "/coding",
    "/claude",
];

pub async fn fetch_models(
    base_url: &str,
    api_key: &str,
    is_full_url: bool,
    models_url_override: Option<&str>,
    api_format: Option<&str>,
) -> Result<Vec<FetchedModel>, String> {
    if api_key.is_empty() {
        return Err("API Key is required to fetch models".to_string());
    }

    let anthropic_style = api_format
        .map(|f| f.eq_ignore_ascii_case("anthropic"))
        .unwrap_or(false);

    let candidates = build_models_url_candidates(base_url, is_full_url, models_url_override)?;
    let client = reqwest::Client::new();
    let mut last_err: Option<String> = None;

    for url in &candidates {
        tracing::debug!("[ModelFetch] Trying endpoint: {url}");
        let mut request = client
            .get(url)
            .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS));
        request = if anthropic_style {
            request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
        } else {
            request.header("Authorization", format!("Bearer {api_key}"))
        };
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                return Err(format!("Request failed: {e}"));
            }
        };

        let status = response.status();

        if status.is_success() {
            // 反爬拦截兜底：部分站点（如 OpenRouter 根路径 /v1/models）会返回
            // 200 + HTML 页面，直接解析会得到误导性错误；跳过当前候选继续尝试。
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !content_type.is_empty() && !content_type.contains("json") {
                tracing::debug!("[ModelFetch] Non-JSON response ({content_type}) from {url}, trying next candidate");
                last_err = Some(format!(
                    "endpoint {url} returned non-JSON response ({content_type}), likely an anti-bot page"
                ));
                continue;
            }

            let resp: ModelsResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            let mut models: Vec<FetchedModel> = resp
                .data
                .unwrap_or_default()
                .into_iter()
                .filter_map(|m| {
                    let id = if m.id.is_empty() {
                        m.display_name.unwrap_or_default()
                    } else {
                        m.id
                    };
                    if id.is_empty() {
                        return None;
                    }
                    Some(FetchedModel {
                        id,
                        owned_by: m.owned_by,
                    })
                })
                .collect();

            models.sort_by(|a, b| a.id.cmp(&b.id));
            return Ok(models);
        }

        if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
            let body = truncate_body(response.text().await.unwrap_or_default());
            last_err = Some(format!("HTTP {status}: {body}"));
            continue;
        }

        let body = truncate_body(response.text().await.unwrap_or_default());
        return Err(format!("HTTP {status}: {body}"));
    }

    Err(format!(
        "All candidates failed: {}",
        last_err.unwrap_or_else(|| "no candidates".to_string())
    ))
}

pub fn build_models_url_candidates(
    base_url: &str,
    is_full_url: bool,
    models_url_override: Option<&str>,
) -> Result<Vec<String>, String> {
    if let Some(raw) = models_url_override {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(vec![trimmed.to_string()]);
        }
    }

    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Base URL is empty".to_string());
    }

    let mut candidates: Vec<String> = Vec::new();

    // OpenRouter 特例：聊天地址是 /api/v1/chat/completions，但 /v1/models 根路径
    // 会被反爬拦截返回 HTML，真实模型列表在 /api/v1/models，优先尝试。
    if host_matches(trimmed, "openrouter.ai") {
        candidates.push("https://openrouter.ai/api/v1/models".to_string());
    }

    if is_full_url {
        // 完整地址推导模型列表端点（用户可能填完整端点，也可能只填到域名）：
        // 1. 含 /v1/ → 取 host + 路径前缀拼 /v1/models（如 https://h.com/api/v1/messages → https://h.com/api/v1/models）；
        // 2. 命中兼容后缀（/anthropic 等）→ 去后缀后再拼；
        // 3. 有一个以上路径段 → 去末段拼 /v1/models；
        // 4. 只有域名（无路径）→ 直接在其上拼候选。
        // 始终至少产出候选，拒绝推导会让"完整填写"模式无法获取模型。
        if let Some(idx) = trimmed.find("/v1/") {
            let prefix = trimmed[..idx].trim_end_matches('/');
            candidates.push(format!("{prefix}/v1/models"));
            if let Some(stripped) = strip_compat_suffix(prefix) {
                let root = stripped.trim_end_matches('/');
                if !root.is_empty() && root.contains("://") {
                    candidates.push(format!("{root}/v1/models"));
                }
            }
        } else if let Some(idx) = trimmed.rfind('/') {
            let root = &trimmed[..idx];
            if root.contains("://") && root.len() > root.find("://").unwrap() + 3 {
                candidates.push(format!("{root}/v1/models"));
                if let Some(stripped) = strip_compat_suffix(root) {
                    let bare = stripped.trim_end_matches('/');
                    if !bare.is_empty() && bare != root {
                        candidates.push(format!("{bare}/v1/models"));
                    }
                }
            } else {
                // 只有 scheme://host（无路径段）：直接在其上拼候选。
                candidates.push(format!("{trimmed}/v1/models"));
                candidates.push(format!("{trimmed}/models"));
            }
        } else {
            // 连 / 都没有的极端输入也尽力尝试。
            candidates.push(format!("{trimmed}/v1/models"));
        }

        let mut unique: Vec<String> = Vec::with_capacity(candidates.len());
        for url in candidates {
            if !unique.iter().any(|u| u == &url) {
                unique.push(url);
            }
        }
        return Ok(unique);
    }

    if ends_with_version_segment(trimmed) {
        candidates.push(format!("{trimmed}/models"));
        if !trimmed.ends_with("/v1") {
            candidates.push(format!("{trimmed}/v1/models"));
        }
    } else {
        candidates.push(format!("{trimmed}/v1/models"));
    }

    if let Some(stripped) = strip_compat_suffix(trimmed) {
        let root = stripped.trim_end_matches('/');
        if !root.is_empty() && root.contains("://") {
            candidates.push(format!("{root}/v1/models"));
            candidates.push(format!("{root}/models"));
        }
    }

    let mut unique: Vec<String> = Vec::with_capacity(candidates.len());
    for url in candidates {
        if !unique.iter().any(|u| u == &url) {
            unique.push(url);
        }
    }

    Ok(unique)
}

fn truncate_body(body: String) -> String {
    if body.chars().count() <= ERROR_BODY_MAX_CHARS {
        body
    } else {
        let mut s: String = body.chars().take(ERROR_BODY_MAX_CHARS).collect();
        s.push('…');
        s
    }
}

fn strip_compat_suffix(base_url: &str) -> Option<&str> {
    for suffix in KNOWN_COMPAT_SUFFIXES {
        if base_url.ends_with(*suffix) {
            return Some(&base_url[..base_url.len() - suffix.len()]);
        }
    }
    None
}

fn ends_with_version_segment(url: &str) -> bool {
    let last = url.rsplit('/').next().unwrap_or("");
    last.strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// 判断 URL 的 host 是否为指定域名（含子域名）。
fn host_matches(url: &str, domain: &str) -> bool {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or("").to_ascii_lowercase();
    let bare = host.rsplit('@').next().unwrap_or(&host);
    let host_only = bare.split(':').next().unwrap_or(bare);
    host_only == domain || host_only.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full(url: &str) -> Vec<String> {
        build_models_url_candidates(url, true, None).expect("full-url derivation must not fail")
    }

    #[test]
    fn full_url_with_v1_endpoint_derives_models_path() {
        assert_eq!(
            full("https://relay.example.com/api/v1/messages"),
            vec!["https://relay.example.com/api/v1/models"]
        );
    }

    #[test]
    fn full_url_with_compat_suffix_also_tries_host_root() {
        let candidates = full("https://relay.example.com/anthropic/v1/messages");
        assert!(candidates.contains(&"https://relay.example.com/anthropic/v1/models".to_string()));
        assert!(candidates.contains(&"https://relay.example.com/v1/models".to_string()));
    }

    #[test]
    fn full_url_without_v1_strips_last_segment() {
        assert_eq!(
            full("https://relay.example.com/some/endpoint"),
            vec!["https://relay.example.com/some/v1/models"]
        );
    }

    #[test]
    fn full_url_host_only_never_errors() {
        // 旧实现在这里直接报 "Cannot derive models endpoint from full URL"。
        assert_eq!(
            full("https://relay.example.com"),
            vec![
                "https://relay.example.com/v1/models",
                "https://relay.example.com/models",
            ]
        );
    }

    #[test]
    fn override_wins_over_derivation() {
        assert_eq!(
            build_models_url_candidates("https://ignored.example.com", true, Some("https://x.com/m"))
                .expect("override"),
            vec!["https://x.com/m"]
        );
    }

    #[test]
    fn non_full_url_keeps_existing_behavior() {
        assert_eq!(
            build_models_url_candidates("https://api.openai.com/v1", false, None)
                .expect("base url"),
            vec!["https://api.openai.com/v1/models"]
        );
    }
}
