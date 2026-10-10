use reqwest::Client;
use std::time::Duration;
use tauri::AppHandle;

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
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let settings = crate::command::settings::get_settings(app.clone())?;
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
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<(ProviderTurnResult, ProviderPromptEstimate), ProviderTurnError> {
        let mut url = self.base_url.trim_end_matches('/').to_string();

        let provider_name = self.adapter.provider_name();
        if provider_name == "openai"
            && !url.ends_with("/v1/chat/completions")
            && !url.ends_with("/chat/completions")
        {
            if url.ends_with("/v1") {
                url = format!("{}/chat/completions", url);
            } else {
                url = format!("{}/v1/chat/completions", url);
            }
        } else if provider_name == "responses"
            && !url.ends_with("/v1/responses")
            && !url.ends_with("/responses")
        {
            if url.ends_with("/v1") {
                url = format!("{}/responses", url);
            } else {
                url = format!("{}/v1/responses", url);
            }
        } else if provider_name == "anthropic"
            && !url.ends_with("/v1/messages")
            && !url.ends_with("/messages")
        {
            if url.ends_with("/v1") {
                url = format!("{}/messages", url);
            } else {
                url = format!("{}/v1/messages", url);
            }
        }

        let builder = self.http_client.post(&url);
        let (req_builder, estimate) =
            self.adapter
                .build_request(builder, app, messages, agent_mode, conversation_id)?;

        let request = req_builder
            .build()
            .map_err(|e| ProviderTurnError::new(e.to_string()))?;

        // wire 级请求报文写入会话事件日志（轨迹面板调试用，含完整 system prompt/tools/消息数组）。
        if let Some(conv_id) = conversation_id {
            if let Some(body) = request.body() {
                if let Some(bytes) = body.as_bytes() {
                    if let Ok(wire) = std::str::from_utf8(bytes) {
                        let body_value = serde_json::from_str::<serde_json::Value>(wire)
                            .unwrap_or(serde_json::Value::String(wire.to_string()));
                        let event = crate::agent::session::SessionEvent::WireRequest {
                            url: url.clone(),
                            body: body_value,
                        };
                        if let Err(error) =
                            crate::agent::session::append_event(app, conv_id, None, &event).await
                        {
                            tracing::warn!(error = %error, "wire_request event append failed");
                        }
                    }
                }
            }
        }

        let cancel_token = cancellation::get_token(conversation_id);

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

        match resp {
            Ok(res) => {
                if !res.status().is_success() {
                    let status = res.status();
                    let error_text = res.text().await.unwrap_or_default();
                    let msg = format!("API Error [{}] {} => {}", status, url, error_text);
                    emit_backend_error(
                        app,
                        &format!("llm.providers.{}", provider_name),
                        msg.clone(),
                        Some("http.non_success"),
                    );
                    return Err(ProviderTurnError::new(msg));
                }

                let mut parser = AdapterStreamParser {
                    adapter: self.adapter.as_mut(),
                };
                run_streaming(
                    &mut parser,
                    app,
                    res,
                    conversation_id,
                    &self.model,
                    cancel_token,
                )
                .await
                .map(|result| (result, estimate))
            }
            Err(e) => {
                let msg = e.to_string();
                emit_backend_error(
                    app,
                    &format!("llm.providers.{}", provider_name),
                    msg.clone(),
                    Some("http.request"),
                );
                Err(ProviderTurnError::new(msg))
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
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderPromptEstimate, ProviderTurnError> {
        let api_format = self.adapter.provider_name();
        match api_format {
            "anthropic" => crate::provider::adapters::anthropic::estimate_prompt_tokens(
                app,
                messages,
                agent_mode,
                conversation_id,
            ),
            "responses" => crate::provider::adapters::responses::estimate_prompt_tokens(
                app,
                messages,
                agent_mode,
                conversation_id,
            ),
            _ => crate::provider::adapters::openai::estimate_prompt_tokens(
                app,
                messages,
                agent_mode,
                conversation_id,
            ),
        }
    }

    /// 执行一次不携带任何工具的纯净单次推理（用于会话压缩总结、元数据提炼等内部任务，直接使用当前 LLM）
    pub async fn complete_text(
        &mut self,
        app: &AppHandle,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, String> {
        let settings = crate::command::settings::get_settings(app.clone())
            .map_err(|e| format!("加载配置失败: {}", e))?;
        let profile = settings.active_provider_profile();
        let provider_name = self.adapter.provider_name();

        let mut url = self.base_url.trim_end_matches('/').to_string();
        if provider_name == "openai" {
            if !url.ends_with("/v1/chat/completions") && !url.ends_with("/chat/completions") {
                if url.ends_with("/v1") {
                    url = format!("{}/chat/completions", url);
                } else {
                    url = format!("{}/v1/chat/completions", url);
                }
            }
        } else if provider_name == "responses" {
            if !url.ends_with("/v1/responses") && !url.ends_with("/responses") {
                if url.ends_with("/v1") {
                    url = format!("{}/responses", url);
                } else {
                    url = format!("{}/v1/responses", url);
                }
            }
        } else if provider_name == "anthropic" {
            if !url.ends_with("/v1/messages") && !url.ends_with("/messages") {
                if url.ends_with("/v1") {
                    url = format!("{}/messages", url);
                } else {
                    url = format!("{}/v1/messages", url);
                }
            }
        }

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

