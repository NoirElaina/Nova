use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::AppHandle;
use url::Url;

// 返回 web_fetch 的注册信息。
// `read_only=true`，因为它只抓网页内容，不会修改本地状态。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回模型可见的 web_fetch 元数据。
// 模型需要提供 `url`，工具再去抓取该地址的正文内容。
pub fn tool() -> Tool {
    Tool {
        name: "web_fetch".into(),
        description: "Fetch the main textual content of a web page URL.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "HTTP/HTTPS URL to fetch" }
            },
            "required": ["url"]
        }),
    }
}

// 把抓到的正文按字节裁剪到安全长度。
// `max_bytes` 限制返回大小，`boundary` 用来退到合法 UTF-8 边界，避免截断多字节字符。
fn truncate(s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }

    let mut boundary = max_bytes;
    while !s.is_char_boundary(boundary) {
        boundary -= 1;
    }

    format!("{}\n...(truncated)", &s[..boundary])
}

fn parse_http_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|e| format!("Invalid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(format!("Unsupported URL scheme '{scheme}', expected http or https")),
    }
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_fetch(input).await })
}

async fn execute_fetch(input: Value) -> Result<ToolOutcome, ToolFailure> {
    let Some(raw_url) = input.get("url").and_then(|v| v.as_str()) else {
        return Err(ToolFailure::invalid_input("Missing 'url' argument"));
    };

    let url = match parse_http_url(raw_url) {
        Ok(url) => url,
        Err(e) => return Err(ToolFailure::invalid_input(e)),
    };

    let client = match reqwest::Client::builder()
        .redirect(Policy::limited(10))
        .timeout(Duration::from_secs(20))
        .user_agent("Nova/0.1 web_fetch")
        .build()
    {
        Ok(client) => client,
        Err(e) => return Err(ToolFailure::new(format!("Failed to create HTTP client: {e}"))),
    };

    let response = match client.get(url.clone()).send().await {
        Ok(response) => response,
        Err(e) => return Err(ToolFailure::new(format!("Failed to fetch URL: {e}"))),
    };

    let status = response.status();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let body = match response.text().await {
        Ok(text) => text,
        Err(e) => return Err(ToolFailure::new(format!("Failed to read response body: {e}"))),
    };
    let truncated = body.len() > 12000;
    let content = if body.trim().is_empty() {
        String::new()
    } else {
        truncate(body, 12000)
    };

    if !status.is_success() {
        return Err(ToolFailure::new(format!(
            "HTTP {} fetching {}: {}",
            status.as_u16(),
            final_url,
            content
        )));
    }

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "url": url.as_str(),
        "finalUrl": final_url,
        "status": status.as_u16(),
        "contentType": content_type,
        "truncated": truncated,
        "content": content
    })))
}
