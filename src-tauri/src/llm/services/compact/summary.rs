use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use crate::llm::types::{
    AnthropicRequest, AnthropicResponse, Content, ContentBlock, Message, Role,
};

const SUMMARY_MAX_TOKENS: u32 = 1200;
const MAX_SUMMARY_RETRIES: usize = 3;
const COMPACT_SUMMARY_SYSTEM_PROMPT: &str = "You compress prior conversation history for a coding agent. Produce a concise continuation summary that preserves: current goal, concrete decisions, open questions, important tool outcomes, files touched, and any user constraints. Prefer bullet points. Do not include markdown code fences, apology text, or commentary about being a summarizer.";

#[derive(Debug, Serialize)]
struct OpenAiSummaryRequest {
    model: String,
    messages: Vec<OpenAiSummaryMessage>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiSummaryMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiSummaryResponse {
    choices: Vec<OpenAiSummaryChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiSummaryChoice {
    message: OpenAiSummaryMessageContent,
}

#[derive(Debug, Deserialize)]
struct OpenAiSummaryMessageContent {
    content: Option<Value>,
}

fn strip_images_to_placeholders(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .map(|message| {
            let content = match &message.content {
                Content::Text(text) => Content::Text(text.clone()),
                Content::Blocks(blocks) => {
                    Content::Blocks(blocks.iter().map(strip_block_to_placeholder).collect())
                }
            };

            Message {
                role: message.role.clone(),
                content,
            }
        })
        .collect()
}

fn strip_block_to_placeholder(block: &ContentBlock) -> ContentBlock {
    match block {
        ContentBlock::Image { .. } => ContentBlock::Text {
            text: "[image omitted for compact summary]".to_string(),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            is_error,
            content,
        } => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            is_error: *is_error,
            content: content.iter().map(strip_block_to_placeholder).collect(),
        },
        _ => block.clone(),
    }
}

fn render_message_for_summary(message: &Message) -> String {
    let role = match message.role {
        Role::User => "User",
        Role::Assistant => "Nova",
    };

    let mut lines = Vec::new();
    match &message.content {
        Content::Text(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                lines.push(trimmed.to_string());
            }
        }
        Content::Blocks(blocks) => {
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            lines.push(trimmed.to_string());
                        }
                    }
                    ContentBlock::Thinking { .. } => {}
                    ContentBlock::Image { .. } => {
                        lines.push("[image omitted for compact summary]".to_string());
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        lines.push(format!("Tool call: {} {}", name, input));
                    }
                    ContentBlock::ToolResult {
                        is_error, content, ..
                    } => {
                        let mut result_lines = Vec::new();
                        for inner in content {
                            match inner {
                                ContentBlock::Text { text } => {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        result_lines.push(trimmed.to_string());
                                    }
                                }
                                ContentBlock::Image { .. } => {
                                    result_lines
                                        .push("[image omitted for compact summary]".to_string());
                                }
                                _ => {}
                            }
                        }
                        if !result_lines.is_empty() {
                            lines.push(format!(
                                "Tool result ({}): {}",
                                if *is_error { "error" } else { "ok" },
                                result_lines.join(" | ")
                            ));
                        }
                    }
                }
            }
        }
    }

    format!("{}: {}", role, lines.join("\n"))
}

fn render_summary_transcript(messages: &[Message]) -> String {
    messages
        .iter()
        .map(render_message_for_summary)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_summary_user_prompt(messages: &[Message]) -> String {
    format!(
        "Summarize the earlier conversation history below so the coding agent can continue seamlessly.\n\nPreserve:\n- current objective\n- confirmed decisions\n- files, tools, and outputs that still matter\n- unresolved issues and next steps\n- user constraints or preferences\n\nConversation transcript:\n{}",
        render_summary_transcript(messages)
    )
}

fn extract_text_from_openai_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn normalize_openai_url(base_url: &str) -> String {
    let mut url = base_url.trim_end_matches('/').to_string();
    if !url.ends_with("/v1/chat/completions") && !url.ends_with("/chat/completions") {
        if url.ends_with("/v1") {
            url = format!("{}/chat/completions", url);
        } else {
            url = format!("{}/v1/chat/completions", url);
        }
    }
    url
}

fn normalize_anthropic_url(base_url: &str) -> String {
    let mut url = base_url.trim_end_matches('/').to_string();
    if !url.ends_with("/v1/messages") && !url.ends_with("/messages") {
        if url.ends_with("/v1") {
            url = format!("{}/messages", url);
        } else {
            url = format!("{}/v1/messages", url);
        }
    }
    url
}

async fn summarize_with_anthropic(app: &AppHandle, user_prompt: &str) -> Result<String, String> {
    let settings = crate::command::settings::get_settings(app.clone());
    let profile = settings.active_provider_profile();
    let api_key = profile.api_key.clone();
    if api_key.is_empty() {
        return Err("API error: No API key configured. Please set it in Settings.".to_string());
    }

    let request = AnthropicRequest {
        model: profile.model.clone(),
        max_tokens: SUMMARY_MAX_TOKENS,
        system: Some(COMPACT_SUMMARY_SYSTEM_PROMPT.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: Content::Text(user_prompt.to_string()),
        }],
        tools: Vec::new(),
        stream: false,
    };

    let client = Client::new();
    let url = normalize_anthropic_url(&profile.base_url);
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("API Error [{}] {} => {}", status, url, error_text));
    }

    let parsed = response
        .json::<AnthropicResponse>()
        .await
        .map_err(|e| e.to_string())?;
    let summary = parsed
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if summary.trim().is_empty() {
        return Err("Compaction summary response did not contain text".to_string());
    }

    Ok(summary)
}

async fn summarize_with_openai(app: &AppHandle, user_prompt: &str) -> Result<String, String> {
    let settings = crate::command::settings::get_settings(app.clone());
    let profile = settings.active_provider_profile();
    let request = OpenAiSummaryRequest {
        model: profile.model.clone(),
        messages: vec![
            OpenAiSummaryMessage {
                role: "system".to_string(),
                content: COMPACT_SUMMARY_SYSTEM_PROMPT.to_string(),
            },
            OpenAiSummaryMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            },
        ],
        max_tokens: SUMMARY_MAX_TOKENS,
        stream: false,
    };

    let client = Client::new();
    let url = normalize_openai_url(&profile.base_url);
    let mut request_builder = client.post(&url).header("content-type", "application/json");
    if !profile.api_key.is_empty() {
        request_builder =
            request_builder.header("Authorization", format!("Bearer {}", profile.api_key));
    }

    let response = request_builder
        .json(&request)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("API Error [{}] {} => {}", status, url, error_text));
    }

    let parsed = response
        .json::<OpenAiSummaryResponse>()
        .await
        .map_err(|e| e.to_string())?;
    let summary = parsed
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .map(extract_text_from_openai_content)
        .unwrap_or_default();
    if summary.trim().is_empty() {
        return Err("Compaction summary response did not contain text".to_string());
    }

    Ok(summary)
}

fn is_prompt_too_long_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    [
        "prompt_too_long",
        "prompt too long",
        "context length",
        "context too long",
        "maximum context length",
        "context window",
        "too many tokens",
        "token limit exceeded",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn truncate_oldest_summary_messages(messages: &[Message]) -> Vec<Message> {
    if messages.len() <= 4 {
        return messages.to_vec();
    }

    let drop_count = ((messages.len() as f32) * 0.2).ceil() as usize;
    let drop_count = drop_count.clamp(1, messages.len().saturating_sub(2));
    messages[drop_count..].to_vec()
}

pub(crate) async fn summarize_messages_for_compact(
    app: &AppHandle,
    messages: &[Message],
) -> Result<String, String> {
    let settings = crate::command::settings::get_settings(app.clone());
    let provider_protocol = settings.active_provider_protocol();
    let mut working_messages = strip_images_to_placeholders(messages);

    for attempt in 0..=MAX_SUMMARY_RETRIES {
        let user_prompt = build_summary_user_prompt(&working_messages);
        let result = if provider_protocol == "anthropic" {
            summarize_with_anthropic(app, &user_prompt).await
        } else {
            summarize_with_openai(app, &user_prompt).await
        };

        match result {
            Ok(summary) => return Ok(summary),
            Err(error) if attempt < MAX_SUMMARY_RETRIES && is_prompt_too_long_error(&error) => {
                let truncated = truncate_oldest_summary_messages(&working_messages);
                if truncated.len() == working_messages.len() {
                    return Err(error);
                }
                working_messages = truncated;
            }
            Err(error) => return Err(error),
        }
    }

    Err("Compaction summary failed after retries".to_string())
}
