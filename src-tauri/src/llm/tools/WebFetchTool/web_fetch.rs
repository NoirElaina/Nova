use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use url::Url;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "WebFetch".into(),
        description: r#"Fetch a URL, convert the page to markdown, and answer `prompt` against it.

- `url`: the URL to fetch (required). HTTP is upgraded to HTTPS.
- `prompt`: the prompt to run on the fetched content.

Fails on authenticated/private URLs. Cross-host redirects are returned rather than followed. Responses are cached for 15 minutes per URL."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from",
                    "format": "uri"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to run on the fetched content"
                }
            },
            "required": ["url"]
        }),
    }
}

const MAX_BODY_BYTES: usize = 512 * 1024;
const CACHE_TTL_SECS: u64 = 900;
// 缓存条目上限：超过时淘汰最旧条目，避免长会话无界增长导致 OOM。
// 64 条 × 512KB 上限 ≈ 32MB 内存占用上限。
const CACHE_MAX_ENTRIES: usize = 64;

struct CacheEntry {
    content: String,
    content_type: String,
    final_url: String,
    fetched_at: Instant,
}

static FETCH_CACHE: std::sync::LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(input).await })
}

async fn execute_async(input: Value) -> Result<ToolOutcome, ToolFailure> {
    let raw_url = input
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing 'url' argument"))?;

    let url_str = if raw_url.starts_with("http://") {
        format!("https://{}", &raw_url[7..])
    } else if !raw_url.starts_with("https://") {
        format!("https://{}", raw_url)
    } else {
        raw_url.to_string()
    };

    let url = Url::parse(&url_str)
        .map_err(|e| ToolFailure::invalid_input(format!("Invalid URL: {e}")))?;

    // Check cache.
    {
        let cache = FETCH_CACHE.lock().unwrap();
        if let Some(entry) = cache.get(url.as_str()) {
            if entry.fetched_at.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
                return format_fetch_result(url.as_str(), &entry.final_url, &entry.content_type, &entry.content, input.get("prompt").and_then(Value::as_str));
            }
        }
    }

    let client = reqwest::Client::builder()
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.stop()
            } else if attempt.previous().last().and_then(|u| u.host_str())
                != attempt.url().host_str()
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .timeout(Duration::from_secs(20))
        .user_agent("Nova/0.1 WebFetch")
        .build()
        .map_err(|e| ToolFailure::new(format!("Failed to create HTTP client: {e}")))?;

    let response = client.get(url.clone()).send().await.map_err(|e| {
        // Check if we got a redirect response that was stopped.
        ToolFailure::new(format!("Failed to fetch URL: {e}"))
    })?;

    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html")
        .to_string();

    let body = response
        .text()
        .await
        .map_err(|e| ToolFailure::new(format!("Failed to read response body: {e}")))?;

    let truncated = body.len() > MAX_BODY_BYTES;
    let body = if truncated {
        let mut boundary = MAX_BODY_BYTES;
        while !body.is_char_boundary(boundary) {
            boundary -= 1;
        }
        body[..boundary].to_string()
    } else {
        body
    };

    let content = html_to_markdown(&body);

    // Update cache.
    {
        let mut cache = FETCH_CACHE.lock().unwrap();
        // 容量上限：达到上限时淘汰 fetched_at 最旧的条目。
        if cache.len() >= CACHE_MAX_ENTRIES {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(
            url.to_string(),
            CacheEntry {
                content: content.clone(),
                content_type: content_type.clone(),
                final_url: final_url.clone(),
                fetched_at: Instant::now(),
            },
        );
    }

    format_fetch_result(url.as_str(), &final_url, &content_type, &content, input.get("prompt").and_then(Value::as_str))
}

fn format_fetch_result(
    original_url: &str,
    final_url: &str,
    _content_type: &str,
    content: &str,
    prompt: Option<&str>,
) -> Result<ToolOutcome, ToolFailure> {
    let mut output = String::new();

    if final_url != original_url {
        output.push_str(&format!("Fetched URL: {}\nRedirected to: {}\n\n", original_url, final_url));
    } else {
        output.push_str(&format!("Fetched URL: {}\n\n", final_url));
    }

    if let Some(p) = prompt {
        output.push_str(&format!("Prompt: {}\n\n", p));
    }

    output.push_str(content);

    Ok(ToolOutcome::text(output))
}

fn html_to_markdown(html: &str) -> String {
    // Strip script and style blocks.
    let mut cleaned = String::new();
    let mut skip = false;
    let mut skip_tag = "";

    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if skip {
            let end_tag = format!("</{}>", skip_tag);
            let remaining: String = chars[i..].iter().collect();
            if remaining.to_lowercase().starts_with(&end_tag) {
                i += end_tag.len();
                skip = false;
            } else {
                i += 1;
            }
            continue;
        }

        if i + 7 < chars.len() {
            let slice: String = chars[i..i + 7].iter().collect();
            if slice.to_lowercase() == "<script" {
                skip = true;
                skip_tag = "script";
                i += 7;
                continue;
            }
        }
        if i + 6 < chars.len() {
            let slice: String = chars[i..i + 6].iter().collect();
            if slice.to_lowercase() == "<style" {
                skip = true;
                skip_tag = "style";
                i += 6;
                continue;
            }
        }

        cleaned.push(chars[i]);
        i += 1;
    }

    // Basic HTML-to-text conversion.
    let text = cleaned
        .replace("</p>", "\n\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</h1>", "\n\n# ")
        .replace("</h2>", "\n\n## ")
        .replace("</h3>", "\n\n### ")
        .replace("</h4>", "\n\n#### ")
        .replace("</li>", "\n")
        .replace("</tr>", "\n")
        .replace("</div>", "\n");

    // Strip remaining HTML tags.
    let mut result = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode common HTML entities.
    let decoded = result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ");

    // Collapse consecutive blank lines.
    let lines: Vec<&str> = decoded.lines().collect();
    let mut clean = String::new();
    let mut prev_blank = false;
    for line in lines {
        let blank = line.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        prev_blank = blank;
        clean.push_str(line);
        clean.push('\n');
    }

    clean.trim().to_string()
}
