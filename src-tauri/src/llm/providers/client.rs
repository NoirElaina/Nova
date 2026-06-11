use reqwest::Client;
use tauri::AppHandle;

use crate::llm::cancellation::is_cancelled;
use crate::llm::providers::adapters::{
    anthropic::AnthropicAdapter, openai::OpenAiAdapter, responses::ResponsesAdapter, ApiAdapter,
};
use crate::llm::providers::stream_runner::{run_streaming, StreamParser};
use crate::llm::providers::{ProviderPromptEstimate, ProviderTurnError, ProviderTurnResult};
use crate::llm::types::{AgentMode, Message};
use crate::llm::utils::error_event::emit_backend_error;

pub struct LlmClient {
    adapter: Box<dyn ApiAdapter>,
    base_url: String,
    model: String,
}

// 桥接 ApiAdapter 到 StreamParser 以便复用 run_streaming
struct AdapterStreamParser<'a> {
    adapter: &'a mut dyn ApiAdapter,
}

impl<'a> StreamParser for AdapterStreamParser<'a> {
    fn parse_event(
        &mut self,
        data: &str,
    ) -> Result<Vec<crate::llm::providers::stream_runner::Delta>, String> {
        self.adapter.parse_event(data)
    }

    fn flush(&mut self) -> Vec<crate::llm::providers::stream_runner::Delta> {
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

        Ok(Self {
            adapter,
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
        })
    }

    pub async fn send_request(
        &mut self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderTurnResult, ProviderTurnError> {
        let client = Client::new();
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

        let builder = client.post(&url);
        let req_builder =
            self.adapter
                .build_request(builder, app, messages, agent_mode, conversation_id)?;

        let request = req_builder
            .build()
            .map_err(|e| ProviderTurnError::new(e.to_string()))?;

        if let Some(body) = request.body() {
            if let Some(bytes) = body.as_bytes() {
                if let Ok(wire) = std::str::from_utf8(bytes) {
                    crate::llm::utils::turn_log::log_wire_request(
                        app,
                        conversation_id,
                        &url,
                        wire,
                    );
                }
            }
        }

        let resp = tokio::select! {
            res = client.execute(request) => res,
            _ = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    if is_cancelled(conversation_id) { break; }
                }
            } => {
                return Ok(ProviderTurnResult {
                    messages: Vec::new(),
                    stop_reason: Some("cancelled".into()),
                    input_tokens: None,
                    output_tokens: None,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    cost: None,
                    prevent_continuation: false,
                });
            }
        };

        match resp {
            Ok(res) => {
                if !res.status().is_success() {
                    let status = res.status();
                    let error_text = res.text().await.unwrap_or_default();
                    eprintln!("API Error: {}", error_text);
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
                run_streaming(&mut parser, app, res, conversation_id, &self.model).await
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

    pub fn estimate_prompt_tokens(
        &self,
        app: &AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<ProviderPromptEstimate, ProviderTurnError> {
        let api_format = self.adapter.provider_name();
        match api_format {
            "anthropic" => crate::llm::providers::adapters::anthropic::estimate_prompt_tokens(
                app,
                messages,
                agent_mode,
                conversation_id,
            ),
            "responses" => crate::llm::providers::adapters::responses::estimate_prompt_tokens(
                app,
                messages,
                agent_mode,
                conversation_id,
            ),
            _ => crate::llm::providers::adapters::openai::estimate_prompt_tokens(
                app,
                messages,
                agent_mode,
                conversation_id,
            ),
        }
    }
}
