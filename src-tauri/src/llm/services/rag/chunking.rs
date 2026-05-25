pub const DEFAULT_CHUNK_SIZE: usize = 900;
pub const DEFAULT_CHUNK_OVERLAP: usize = 120;
pub const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_BATCH_SIZE: usize = 200;

pub fn normalize_content(raw: &str) -> String {
    raw.replace("\r\n", "\n").trim().to_string()
}

pub fn normalize_source_type(raw: &str) -> String {
    let key = raw.trim().to_ascii_lowercase();
    if key.is_empty() {
        "text".to_string()
    } else {
        key
    }
}

pub fn normalize_source_name(raw: &str, fallback_index: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        format!("document-{}", fallback_index + 1)
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

pub fn preview_text(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let preview: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{}...", preview)
    } else {
        preview
    }
}

pub fn split_into_chunks(content: &str) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    let total = chars.len();
    if total <= DEFAULT_CHUNK_SIZE {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < total {
        let hard_end = (start + DEFAULT_CHUNK_SIZE).min(total);
        let mut end = hard_end;

        if hard_end < total {
            while end > start && !chars[end - 1].is_whitespace() {
                end -= 1;
            }
            if end == start {
                end = hard_end;
            }
        }

        let chunk: String = chars[start..end].iter().collect();
        let trimmed = chunk.trim().to_string();
        if !trimmed.is_empty() {
            chunks.push(trimmed);
        }

        if end >= total {
            break;
        }

        start = if end > start + DEFAULT_CHUNK_OVERLAP {
            end - DEFAULT_CHUNK_OVERLAP
        } else {
            end
        };
        while start < total && chars[start].is_whitespace() {
            start += 1;
        }
    }

    if chunks.is_empty() {
        chunks.push(content.to_string());
    }
    chunks
}
