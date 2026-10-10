use tauri::AppHandle;

use crate::provider::LlmClient;
use crate::provider::types::{Content, ContentBlock, Message, Role};

const COMPACT_SUMMARY_SYSTEM_PROMPT: &str = "You compress prior conversation history for a coding agent. Produce a concise continuation summary that preserves: current goal, concrete decisions, open questions, important tool outcomes, files touched, and any user constraints. Prefer bullet points. Do not include markdown code fences, apology text, or commentary about being a summarizer.\n\nIMPORTANT: Produce the summary in the predominant language used in the conversation (for example, if the conversation is in Chinese, write the summary in Chinese).";
const MAX_SUMMARY_RETRIES: usize = 2;

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

    let drop_count = ((messages.len() as f32) * 0.25).ceil() as usize;
    let drop_count = drop_count.clamp(1, messages.len().saturating_sub(2));
    messages[drop_count..].to_vec()
}

/// 直接使用当前配置的 LLM 对历史消息进行结构化压缩摘要
pub(crate) async fn summarize_messages_for_compact(
    app: &AppHandle,
    messages: &[Message],
) -> Result<String, String> {
    let mut working_messages = strip_images_to_placeholders(messages);

    for attempt in 0..=MAX_SUMMARY_RETRIES {
        let user_prompt = build_summary_user_prompt(&working_messages);

        // 直接复用当前配置的 LLM 客户端！
        let mut client = LlmClient::new(app)?;
        let result = client
            .complete_text(app, COMPACT_SUMMARY_SYSTEM_PROMPT, &user_prompt)
            .await;

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
