use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::AppHandle;
use url::Url;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "WebSearch".into(),
        description: r#"Search the web. Returns result blocks with titles and URLs.

- `query`: the search query (required, min 2 chars).
- `allowed_domains`: only include search results from these domains.
- `blocked_domains`: never include search results from these domains.

Sources are listed as markdown links at the end of results."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to use",
                    "minLength": 2
                },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only include search results from these domains"
                },
                "blocked_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Never include search results from these domains"
                }
            },
            "required": ["query"]
        }),
    }
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(input).await })
}

async fn execute_async(input: Value) -> Result<ToolOutcome, ToolFailure> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .filter(|v| v.trim().len() >= 2)
        .ok_or_else(|| ToolFailure::invalid_input("Missing or too short 'query' (min 2 chars)"))?;

    let allowed_domains: Vec<String> = input
        .get("allowed_domains")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let blocked_domains: Vec<String> = input
        .get("blocked_domains")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let search_url = format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        urlencoding::encode(query)
    );

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| ToolFailure::new(format!("Failed to create HTTP client: {e}")))?;

    let response = client.get(&search_url).send().await.map_err(|e| {
        ToolFailure::new(format!("Search request failed: {e}"))
    })?;

    let html = response.text().await.map_err(|e| {
        ToolFailure::new(format!("Failed to read search response: {e}"))
    })?;

    let results = parse_duckduckgo_lite(&html, &allowed_domains, &blocked_domains);

    if results.is_empty() {
        return Ok(ToolOutcome::text(format!(
            "No results found for query: \"{}\"",
            query
        )));
    }

    let mut output = String::new();
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url
        ));
    }

    let sources: Vec<String> = results
        .iter()
        .map(|r| format!("- [{}]({})", r.title, r.url))
        .collect();
    output.push_str("Sources:\n");
    output.push_str(&sources.join("\n"));

    Ok(ToolOutcome::text(output))
}

struct SearchResult {
    title: String,
    url: String,
}

fn parse_duckduckgo_lite(html: &str, allowed: &[String], blocked: &[String]) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for line in html.lines() {
        let trimmed = line.trim();

        if trimmed.contains("result-link")
            || (trimmed.starts_with("<a") && trimmed.contains("href="))
        {
            if let Some(url) = extract_href(trimmed) {
                let title = strip_html(trimmed);
                if !title.is_empty() && !url.is_empty() && !is_blocked(&url, allowed, blocked) {
                    results.push(SearchResult { title, url });
                }
            }
        }
    }

    results
}

fn extract_href(html: &str) -> Option<String> {
    let start = html.find("href=\"")?;
    let rest = &html[start + 6..];
    let end = rest.find('"')?;
    let raw = &rest[..end];

    // Decode HTML entities
    let decoded = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");

    // Skip internal DuckDuckGo links and non-http URLs
    if decoded.starts_with("//duckduckgo.com")
        || decoded.starts_with("/")
        || decoded.contains("duckduckgo.com/l/?")
        || decoded.starts_with("javascript:")
    {
        return None;
    }

    // Normalize // links
    if decoded.starts_with("//") {
        Some(format!("https:{}", decoded))
    } else if decoded.starts_with("http://") || decoded.starts_with("https://") {
        Some(decoded)
    } else {
        None
    }
}

fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .trim()
        .to_string()
}

fn is_blocked(url: &str, allowed: &[String], blocked: &[String]) -> bool {
    let host = Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();

    if !allowed.is_empty() {
        return !allowed.iter().any(|d| host.ends_with(d.trim_start_matches("*.")));
    }

    if blocked.iter().any(|d| host.ends_with(d.trim_start_matches("*."))) {
        return true;
    }

    false
}
