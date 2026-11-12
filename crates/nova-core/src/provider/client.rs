use reqwest::Client;
use std::time::Duration;
use crate::core::Core;

use crate::agent::cancellation;
use crate::provider::adapters::{
    anthropic::AnthropicAdapter, openai::OpenAiAdapter, responses::ResponsesAdapter, ApiAdapter,
};
use crate::provider::stream_runner::{run_streaming, StreamParser};
use crate::provider::{ProviderPromptEstimate, ProviderTurnError, ProviderTurnResult};
use crate::provider::types::{AgentMode, Message};
use crate::agent::utils::error_event::emit_backend_error;

pub struct LlmClient {
    adapter: Box<dyn ApiAdapter>,
    base_url: String,
    model: String,
    http_client: Client,
}

const MAX_STREAM_RETRIES: u32 = 3;

pub(crate) fn should_retry_stream_error(
    err: &ProviderTurnError,
    retry_attempt: u32,
    cancelled: bool,
) -> bool {
    err.error_info.is_retryable()
        && err.partial_messages.is_empty()
        && retry_attempt <= MAX_STREAM_RETRIES
        && !cancelled
}

// 桥接 ApiAdapter 到 StreamParser 以便复用 run_streaming
struct AdapterStreamParser<'a> {
    adapter: &'a mut dyn ApiAdapter,
}

impl<'a> StreamParser for AdapterStreamParser<'a> {
    fn parse_event(
        &mut self,
        data: &str,
    ) -> Result<Vec<crate::provider::stream_runner::Delta>, String> {
        self.adapter.parse_event(data)
    }

    fn flush(&mut self) -> Vec<crate::provider::stream_runner::Delta> {
        self.adapter.flush()
    }

    fn provider_name(&self) -> &'static str {
        self.adapter.provider_name()
    }
}

impl LlmClient {
    pub fn new(core: &Core) -> Result<Self, String> {
        let settings = crate::services::settings::load_settings(core)?;
        let profile = settings.active_provider_profile();
        let api_format = profile.api_format.as_str();

        let adapter: Box<dyn ApiAdapter> = match api_format {
            "anthropic" => Box::new(AnthropicAdapter::new()),
            "openai_responses" => Box::new(ResponsesAdapter::new()),
            _ => Box::new(OpenAiAdapter::new()),
        };

        let http_client = Client::builder()
            // read_timeout：连续 120s 收不到任何字节才超时。
            // 不能用 timeout()——那是"连接开始到 body 流完"的总超时，
            // 长轮次（长思考+长输出）超 5 分钟会被从中间掐断，
            // 表现为 stream chunk error: error decoding response body。
            // 健康的 SSE 流有 ping/增量 token，永远不会触发 read_timeout。
            .read_timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        Ok(Self {
            adapter,
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            http_client,
        })
    }

    pub async fn send_request(
        &mut self,
        core: &Core,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<(ProviderTurnResult, ProviderPromptEstimate), ProviderTurnError> {
        let provider_name = self.adapter.provider_name();
        let url = resolve_provider_url(&self.base_url, provider_name)?;

        let cancel_token = cancellation::get_token(conversation_id);
        let mut retry_attempt = 0u32;

        loop {
            retry_attempt += 1;

            let builder = self.http_client.post(&url);
            let (req_builder, estimate) =
                self.adapter
                    .build_request(builder, core, messages, agent_mode, conversation_id)?;

            let request = req_builder
                .build()
                .map_err(|e| ProviderTurnError::new(e.to_string()))?;

            let trace_meta = conversation_id.and_then(|cid| core.active_trace_for_conversation(cid));
            let active_turn_id = trace_meta.as_ref().map(|(tid, _)| tid.clone());
            let active_trace_id = trace_meta.as_ref().map(|(_, tr)| tr.clone());
            let redacted_url = nova_protocol::redact::redact_secrets(&url);

            // wire 级请求报文写入脱敏环形缓冲区，并于首轮落库到会话事件日志。
            if let Some(body) = request.body() {
                if let Some(bytes) = body.as_bytes() {
                    if let Ok(wire) = std::str::from_utf8(bytes) {
                        let raw_value = serde_json::from_str::<serde_json::Value>(wire)
                            .unwrap_or_else(|_| serde_json::Value::String(wire.to_string()));
                        let scrubbed_body = nova_protocol::redact::redact_json_value(&raw_value);
                        core.record_redacted_wire(crate::core::RedactedWireRecord {
                            timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            trace_id: active_trace_id.clone(),
                            conversation_id: conversation_id.map(str::to_string),
                            turn_id: active_turn_id.clone(),
                            provider: provider_name.to_string(),
                            model: self.model.clone(),
                            direction: crate::core::WireDirection::Request,
                            url: Some(redacted_url.clone()),
                            status: None,
                            payload: scrubbed_body.clone(),
                        });
                        if retry_attempt == 1 {
                            if let Some(conv_id) = conversation_id {
                                let event = crate::agent::session::SessionEvent::WireRequest {
                                    url: redacted_url.clone(),
                                    body: scrubbed_body,
                                    trace_id: active_trace_id.clone(),
                                };
                                if let Err(error) = crate::agent::session::append_event(
                                    core,
                                    conv_id,
                                    active_turn_id.as_deref(),
                                    &event,
                                )
                                .await
                                {
                                    tracing::warn!(error = %error, "wire_request event append failed");
                                }
                            }
                        }
                    }
                }
            }

            let resp = tokio::select! {
                res = self.http_client.execute(request) => res,
                _ = cancel_token.cancelled() => {
                    return Ok((ProviderTurnResult {
                        messages: Vec::new(),
                        tool_calls: Vec::new(),
                        stop_reason: Some("cancelled".into()),
                        input_tokens: None,
                        output_tokens: None,
                        cache_read_tokens: None,
                        cache_creation_tokens: None,
                        cost: None,
                        prevent_continuation: false,
                    }, estimate));
                }
            };

            let stream_outcome = match resp {
                Ok(res) => {
                    if !res.status().is_success() {
                        let status = res.status();
                        let error_text = res.text().await.unwrap_or_default();
                        core.record_redacted_wire(crate::core::RedactedWireRecord {
                            timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            trace_id: active_trace_id.clone(),
                            conversation_id: conversation_id.map(str::to_string),
                            turn_id: active_turn_id.clone(),
                            provider: provider_name.to_string(),
                            model: self.model.clone(),
                            direction: crate::core::WireDirection::Error,
                            url: Some(redacted_url.clone()),
                            status: Some(status.as_u16()),
                            payload: serde_json::json!({
                                "status": status.as_u16(),
                                "attempt": retry_attempt,
                                "body": nova_protocol::redact::redact_secrets(&error_text),
                            }),
                        });
                        Err(ProviderTurnError::from_status_and_body(status, &error_text))
                    } else {
                        let mut parser = AdapterStreamParser {
                            adapter: self.adapter.as_mut(),
                        };
                        run_streaming(
                            &mut parser,
                            core,
                            res,
                            conversation_id,
                            &self.model,
                            cancel_token.clone(),
                        )
                        .await
                        .map(|result| (result, estimate))
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    Err(ProviderTurnError::new(msg))
                }
            };

            match stream_outcome {
                Ok(success) => return Ok(success),
                Err(err) => {
                    // 只要本次尝试已经把文本/思考流给了前端，就**不能**重试：
                    // 重发请求会让模型从头再输出一遍，UI 上出现重复段落，
                    // 而历史里只存最终版本，两侧永久不一致。
                    if should_retry_stream_error(&err, retry_attempt, cancel_token.is_cancelled()) {
                        let delay = err.error_info.default_retry_delay(retry_attempt);
                        tracing::warn!(
                            provider = %provider_name,
                            attempt = retry_attempt,
                            max_retries = MAX_STREAM_RETRIES,
                            delay_ms = delay.as_millis(),
                            error = %err.message,
                            "Stream request encountered retryable error; waiting to retry with exponential backoff"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    emit_backend_error(
                        core,
                        &format!("llm.providers.{}", provider_name),
                        err.message.clone(),
                        Some("stream.failed"),
                    );
                    return Err(err);
                }
            }
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn provider_name(&self) -> &'static str {
        self.adapter.provider_name()
    }

    pub fn estimate_prompt_tokens(
        &self,
        core: &Core,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderPromptEstimate, ProviderTurnError> {
        let api_format = self.adapter.provider_name();
        match api_format {
            "anthropic" => crate::provider::adapters::anthropic::estimate_prompt_tokens(
                core,
                messages,
                agent_mode,
                conversation_id,
            ),
            "responses" => crate::provider::adapters::responses::estimate_prompt_tokens(
                core,
                messages,
                agent_mode,
                conversation_id,
            ),
            _ => crate::provider::adapters::openai::estimate_prompt_tokens(
                core,
                messages,
                agent_mode,
                conversation_id,
            ),
        }
    }

    /// 执行一次不携带任何工具的纯净单次推理（用于会话压缩总结、元数据提炼等内部任务，直接使用当前 LLM）
    pub async fn complete_text(
        &mut self,
        core: &Core,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, String> {
        let settings = crate::services::settings::load_settings(core)
            .map_err(|e| format!("加载配置失败: {}", e))?;
        let profile = settings.active_provider_profile();
        let provider_name = self.adapter.provider_name();

        let url = resolve_provider_url(&self.base_url, provider_name)
            .map_err(|e| e.message)?;

        let mut req_builder = self.http_client.post(&url).header("content-type", "application/json");

        let response = match provider_name {
            "anthropic" => {
                if !profile.api_key.is_empty() {
                    req_builder = req_builder
                        .header("x-api-key", &profile.api_key)
                        .header("anthropic-version", "2023-06-01");
                }
                let body = serde_json::json!({
                    "model": self.model,
                    "max_tokens": 1500,
                    "system": system_prompt,
                    "messages": [
                        { "role": "user", "content": user_prompt }
                    ],
                    "stream": false
                });
                let resp = req_builder.json(&body).send().await.map_err(|e| e.to_string())?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    return Err(format!("API Error [{}] {} => {}", status, url, err));
                }
                let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                let content_arr = val.get("content").and_then(|c| c.as_array());
                content_arr
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default()
            }
            "responses" => {
                if !profile.api_key.is_empty() {
                    req_builder = req_builder.header("Authorization", format!("Bearer {}", profile.api_key));
                }
                let body = serde_json::json!({
                    "model": self.model,
                    "instructions": system_prompt,
                    "input": user_prompt,
                    "max_output_tokens": 1500,
                    "stream": false
                });
                let resp = req_builder.json(&body).send().await.map_err(|e| e.to_string())?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    return Err(format!("API Error [{}] {} => {}", status, url, err));
                }
                let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                val.get("output_text")
                    .and_then(|t| t.as_str())
                    .map(str::to_string)
                    .unwrap_or_default()
            }
            _ => {
                if !profile.api_key.is_empty() {
                    req_builder = req_builder.header("Authorization", format!("Bearer {}", profile.api_key));
                }
                let body = serde_json::json!({
                    "model": self.model,
                    "max_tokens": 1500,
                    "messages": [
                        { "role": "system", "content": system_prompt },
                        { "role": "user", "content": user_prompt }
                    ],
                    "stream": false
                });
                let resp = req_builder.json(&body).send().await.map_err(|e| e.to_string())?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    return Err(format!("API Error [{}] {} => {}", status, url, err));
                }
                let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                val.get("choices")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|choice| choice.get("message"))
                    .and_then(|msg| msg.get("content"))
                    .and_then(|cnt| cnt.as_str())
                    .map(str::to_string)
                    .unwrap_or_default()
            }
        };

        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Err("模型未返回有效文本内容".to_string());
        }
        Ok(trimmed.to_string())
    }
}

/// 基于 RFC 3986 标准规范化 Provider API Endpoint，解决网关二级路径与 Query 参数被粗暴截断/重叠的 Bug。
pub(crate) fn resolve_provider_url(
    base_url: &str,
    provider_name: &str,
) -> Result<String, ProviderTurnError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(ProviderTurnError::new("Provider base_url cannot be empty".to_string()));
    }

    let mut parsed = url::Url::parse(trimmed).map_err(|e| {
        ProviderTurnError::new(format!("Invalid provider base_url '{}': {}", trimmed, e))
    })?;

    let target_endpoint = match provider_name {
        "openai" => "chat/completions",
        "responses" => "responses",
        "anthropic" => "messages",
        _ => return Ok(trimmed.to_string()),
    };

    let path = parsed.path().trim_end_matches('/').to_string();

    // 若已经以目标端点结尾（例如 .../chat/completions 或 Azure 完整地址），保留原样
    if path.ends_with(target_endpoint) {
        return Ok(parsed.to_string());
    }

    let new_path = if path.ends_with("/v1") {
        format!("{}/{}", path, target_endpoint)
    } else if path.is_empty() || path == "/" {
        format!("/v1/{}", target_endpoint)
    } else {
        format!("{}/v1/{}", path, target_endpoint)
    };

    parsed.set_path(&new_path);
    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_openai_urls() {
        assert_eq!(
            resolve_provider_url("https://api.openai.com", "openai").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_provider_url("https://api.openai.com/", "openai").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_provider_url("https://api.openai.com/v1", "openai").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_provider_url("https://api.openai.com/v1/", "openai").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_provider_url("https://api.openai.com/v1/chat/completions", "openai").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        // Azure OpenAI 带 query string
        assert_eq!(
            resolve_provider_url(
                "https://my-res.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-02",
                "openai"
            ).unwrap(),
            "https://my-res.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-02"
        );
        // 自定义中转子路径
        assert_eq!(
            resolve_provider_url("https://gateway.internal/ai/proxy/v1", "openai").unwrap(),
            "https://gateway.internal/ai/proxy/v1/chat/completions"
        );
        assert_eq!(
            resolve_provider_url("https://gateway.internal/ai/proxy", "openai").unwrap(),
            "https://gateway.internal/ai/proxy/v1/chat/completions"
        );
    }

    #[test]
    fn test_resolve_anthropic_urls() {
        assert_eq!(
            resolve_provider_url("https://api.anthropic.com", "anthropic").unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            resolve_provider_url("https://api.anthropic.com/v1", "anthropic").unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            resolve_provider_url("https://api.anthropic.com/v1/messages", "anthropic").unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn partial_output_never_retries_even_on_disconnect_error() {
        let retryable_empty =
            ProviderTurnError::new("wsarecv: connection reset (os error 10054)".to_string());
        assert!(should_retry_stream_error(&retryable_empty, 1, false));
        assert!(!should_retry_stream_error(&retryable_empty, 1, true));
        assert!(!should_retry_stream_error(&retryable_empty, MAX_STREAM_RETRIES + 1, false));

        let partial = vec![crate::provider::types::Message {
            role: crate::provider::types::Role::Assistant,
            content: crate::provider::types::Content::Text("partial text".to_string()),
        }];
        let with_partial = ProviderTurnError::with_partial(
            "anthropic stream chunk error: unexpected EOF".to_string(),
            partial,
        );
        assert!(with_partial.error_info.is_retryable());
        assert!(
            !should_retry_stream_error(&with_partial, 1, false),
            "stream error with emitted partial output must never retry"
        );
    }
}

