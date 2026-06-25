use serde_json::Value;

use crate::llm::providers::stream_runner::Delta;

const REASONING_KEYS: &[&str] = &[
    "reasoning_content",
    "reasoning_details",
    "reasoning",
    "thinking_content",
];

const NESTED_TEXT_KEYS: &[&str] = &[
    "text",
    "content",
    "summary",
    "delta",
    "reasoning_content",
    "reasoning_details",
    "reasoning",
    "thinking_content",
];

#[derive(Debug)]
pub(crate) enum InlineThinkPart {
    Text(String),
    Reasoning(String),
}

#[derive(Debug, Default)]
pub(crate) struct InlineThinkExtractor {
    mode: InlineThinkMode,
    pending: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum InlineThinkMode {
    #[default]
    Text,
    Reasoning,
}

pub(crate) fn extract_reasoning_field_text(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let mut parts = Vec::new();

    for key in REASONING_KEYS {
        if let Some(value) = object.get(*key) {
            collect_reasoning_text(value, &mut parts);
        }
    }

    join_parts(parts)
}

pub(crate) fn push_inline_parts(deltas: &mut Vec<Delta>, parts: Vec<InlineThinkPart>) {
    for part in parts {
        match part {
            InlineThinkPart::Text(text) => {
                if !text.is_empty() {
                    deltas.push(Delta::Text(text));
                }
            }
            InlineThinkPart::Reasoning(text) => {
                if !text.is_empty() {
                    deltas.push(Delta::Reasoning(text));
                }
            }
        }
    }
}

impl InlineThinkExtractor {
    pub(crate) fn push(&mut self, text: &str) -> Vec<InlineThinkPart> {
        self.pending.push_str(text);
        self.drain(false)
    }

    pub(crate) fn flush(&mut self) -> Vec<InlineThinkPart> {
        self.drain(true)
    }

    fn drain(&mut self, flush: bool) -> Vec<InlineThinkPart> {
        let mut parts = Vec::new();

        loop {
            if self.mode == InlineThinkMode::Text {
                if let Some(index) = self.pending.find("<think>") {
                    let text = self.pending[..index].to_string();
                    if !text.is_empty() {
                        parts.push(InlineThinkPart::Text(text));
                    }
                    self.pending.drain(..index + "<think>".len());
                    self.mode = InlineThinkMode::Reasoning;
                    continue;
                }

                let emit_len = if flush {
                    self.pending.len()
                } else {
                    safe_emit_len(&self.pending, "<think>")
                };
                if emit_len > 0 {
                    let text: String = self.pending.drain(..emit_len).collect();
                    parts.push(InlineThinkPart::Text(text));
                }
                break;
            }

            if let Some(index) = self.pending.find("</think>") {
                let text = self.pending[..index].to_string();
                if !text.is_empty() {
                    parts.push(InlineThinkPart::Reasoning(text));
                }
                self.pending.drain(..index + "</think>".len());
                self.mode = InlineThinkMode::Text;
                continue;
            }

            let emit_len = if flush {
                self.pending.len()
            } else {
                safe_emit_len(&self.pending, "</think>")
            };
            if emit_len > 0 {
                let text: String = self.pending.drain(..emit_len).collect();
                parts.push(InlineThinkPart::Reasoning(text));
            }
            break;
        }

        parts
    }
}

fn collect_reasoning_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if !text.trim().is_empty() {
                parts.push(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_reasoning_text(item, parts);
            }
        }
        Value::Object(object) => {
            for key in NESTED_TEXT_KEYS {
                if let Some(value) = object.get(*key) {
                    collect_reasoning_text(value, parts);
                }
            }
        }
        _ => {}
    }
}

fn join_parts(parts: Vec<String>) -> Option<String> {
    let text = parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("");

    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn safe_emit_len(buffer: &str, tag: &str) -> usize {
    let max_suffix = tag.len().saturating_sub(1).min(buffer.len());
    for suffix_len in (1..=max_suffix).rev() {
        let start = buffer.len() - suffix_len;
        if !buffer.is_char_boundary(start) {
            continue;
        }
        let suffix = &buffer[start..];
        if tag.starts_with(suffix) {
            return start;
        }
    }
    buffer.len()
}
