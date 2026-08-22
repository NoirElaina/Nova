use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use base64::Engine as _;
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

/// 最多返回的结果条数。
const MAX_RESULTS: usize = 10;
/// 摘要截断长度（字符数）。
const SNIPPET_MAX_CHARS: usize = 220;

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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(15))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )
        .build()
        .map_err(|e| ToolFailure::new(format!("Failed to create HTTP client: {e}")))?;

    // 引擎链：Bing 优先（国内连通性好），失败或零结果时回退 DuckDuckGo Lite。
    // DDG Lite 近期频繁返回机器人验证页（HTTP 202 + anomaly challenge），
    // 单引擎时代这是搜索工具"永远无结果"的直接原因。
    let mut results = match search_bing(&client, query).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("WebSearch: Bing engine failed: {e}");
            Vec::new()
        }
    };

    if results.is_empty() {
        match search_duckduckgo_lite(&client, query).await {
            Ok(r) => results = r,
            Err(e) => tracing::warn!("WebSearch: DuckDuckGo fallback failed: {e}"),
        }
    }

    let results = filter_results(results, &allowed_domains, &blocked_domains);

    if results.is_empty() {
        return Ok(ToolOutcome::text(format!(
            "No results found for query: \"{}\"",
            query
        )));
    }

    let mut output = String::new();
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!("{}. {}\n   {}\n", i + 1, result.title, result.url));
        if let Some(snippet) = &result.snippet {
            output.push_str(&format!("   {}\n", snippet));
        }
        output.push('\n');
    }

    let sources: Vec<String> = results
        .iter()
        .map(|r| format!("- [{}]({})", r.title, r.url))
        .collect();
    output.push_str("Sources:\n");
    output.push_str(&sources.join("\n"));

    Ok(ToolOutcome::text(output))
}

async fn search_bing(client: &reqwest::Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let search_url = format!(
        "https://www.bing.com/search?q={}&count=10",
        urlencoding::encode(query)
    );
    let html = fetch_text(client, &search_url).await?;
    Ok(parse_bing(&html))
}

async fn search_duckduckgo_lite(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    let search_url = format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        urlencoding::encode(query)
    );
    let html = fetch_text(client, &search_url).await?;
    // DDG 对程序化请求返回挑战页（含 anomaly.js）时，页面里没有任何结果链接。
    if html.contains("anomaly") {
        return Err("DuckDuckGo returned a bot challenge page".into());
    }
    Ok(parse_duckduckgo_lite(&html))
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;
    response
        .text()
        .await
        .map_err(|e| format!("Failed to read search response: {e}"))
}

#[derive(Debug)]
struct SearchResult {
    title: String,
    url: String,
    snippet: Option<String>,
}

/// 域名过滤 + URL 去重 + 条数上限。
fn filter_results(
    results: Vec<SearchResult>,
    allowed: &[String],
    blocked: &[String],
) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    results
        .into_iter()
        .filter(|r| seen.insert(r.url.clone()))
        .filter(|r| !is_blocked(&r.url, allowed, blocked))
        .take(MAX_RESULTS)
        .collect()
}

// ---------------- Bing 解析 ----------------

/// 解析 Bing 搜索结果页：每个 `class="b_algo"` 块是一条结果。
/// 标题链接的 href 通常是 `/ck/a?...&u=a1<base64url(真实URL)>` 的跳转包装，
/// 需要解码 `u` 参数才能拿到真实地址。
fn parse_bing(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for raw_block in html.split("class=\"b_algo\"").skip(1) {
        // 单条结果块足够大，限制扫描范围避免误抓页尾推荐链接。
        let block = truncate_utf8(raw_block, 6000);

        let Some(a_start) = block.find("<a ") else {
            continue;
        };
        let anchor = &block[a_start..];
        let Some(href_raw) = extract_attr(anchor, "href=\"") else {
            continue;
        };
        let Some(url) = resolve_bing_href(&decode_entities(&href_raw)) else {
            continue;
        };

        // 标题：优先取块内 <h2>（纯净标题）；Bing 的锚点文本会混入域名面包屑。
        let title = block
            .find("<h2")
            .and_then(|h2_start| {
                block[h2_start..]
                    .find("</h2>")
                    .map(|end| strip_html(&block[h2_start..h2_start + end]))
            })
            .filter(|t| !t.is_empty())
            .or_else(|| {
                anchor
                    .find("</a>")
                    .map(|end| strip_html(&anchor[..end]))
            })
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        // 摘要：块内第一个 <p> 的文本（可能不存在）。
        let snippet = block.find("<p").and_then(|p_start| {
            let rest = &block[p_start..];
            rest.find("</p>").map(|end| {
                truncate_chars(&strip_html(&rest[..end]), SNIPPET_MAX_CHARS)
            })
        });

        results.push(SearchResult {
            title,
            url,
            snippet: snippet.filter(|s| !s.is_empty()),
        });
    }

    results
}

/// 把 Bing 的 href 还原为真实结果地址：
/// - `/ck/a?...&u=a1<base64url>` → 解码 base64url 部分；
/// - 直接是 http(s) 链接 → 原样返回；
/// - 其余（站内链接等） → 丢弃。
fn resolve_bing_href(href: &str) -> Option<String> {
    if href.contains("/ck/a") {
        let encoded = extract_query_value(href, "u=")?;
        let b64 = encoded.strip_prefix("a1")?;
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(b64.trim_end_matches('='))
            .ok()?;
        let real = String::from_utf8(bytes).ok()?;
        if real.starts_with("http://") || real.starts_with("https://") {
            return Some(real);
        }
        return None;
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    None
}

// ---------------- DuckDuckGo Lite 解析 ----------------

fn parse_duckduckgo_lite(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for line in html.lines() {
        let trimmed = line.trim();

        if trimmed.contains("result-link")
            || (trimmed.starts_with("<a") && trimmed.contains("href="))
        {
            if let Some(url) = extract_ddg_href(trimmed) {
                let title = strip_html(trimmed);
                if !title.is_empty() && !url.is_empty() {
                    results.push(SearchResult {
                        title,
                        url,
                        snippet: None,
                    });
                }
            }
        }
    }

    results
}

/// DDG Lite 的结果链接是 `//duckduckgo.com/l/?uddg=<percent-encoded 真实URL>`
/// 的跳转包装：必须解包 `uddg` 参数，而不是把它当内部链接丢弃。
fn extract_ddg_href(html: &str) -> Option<String> {
    let start = html.find("href=\"")?;
    let rest = &html[start + 6..];
    let end = rest.find('"')?;
    let decoded = decode_entities(&rest[..end]);

    if decoded.contains("duckduckgo.com/l/?") {
        let real = extract_query_value(&decoded, "uddg=")?;
        let decoded_real = urlencoding::decode(&real).ok()?.into_owned();
        if decoded_real.starts_with("http://") || decoded_real.starts_with("https://") {
            return Some(decoded_real);
        }
        return None;
    }

    // 跳过站内/非结果链接。
    if decoded.starts_with("//duckduckgo.com")
        || decoded.starts_with('/')
        || decoded.starts_with("javascript:")
    {
        return None;
    }

    if decoded.starts_with("//") {
        Some(format!("https:{}", decoded))
    } else if decoded.starts_with("http://") || decoded.starts_with("https://") {
        Some(decoded.to_string())
    } else {
        None
    }
}

// ---------------- 公共小工具 ----------------

/// 从 `url` 中提取 `key` 对应的查询参数值（截到下一个 `&` 或结尾）。
fn extract_query_value(url: &str, key: &str) -> Option<String> {
    let start = url.find(key)? + key.len();
    let rest = &url[start..];
    let end = rest.find('&').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// 提取属性值：从 `attr_open`（含引号开头，如 `href="`）到下一个引号。
fn extract_attr(html: &str, attr_open: &str) -> Option<String> {
    let start = html.find(attr_open)? + attr_open.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn decode_entities(raw: &str) -> String {
    raw.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
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
    decode_entities(&result)
        .replace("&#x27;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

/// 按字节安全截断（不切断多字节字符）。
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 按字符数截断。
fn truncate_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bing_ck_redirect_is_decoded() {
        // "https://v2.tauri.org.cn/" 的 base64url = aHR0cHM6Ly92Mi50YXVyaS5vcmcuY24v
        let href = "https://www.bing.com/ck/a?!&&p=abc&u=a1aHR0cHM6Ly92Mi50YXVyaS5vcmcuY24v&ntb=1";
        assert_eq!(
            resolve_bing_href(href),
            Some("https://v2.tauri.org.cn/".to_string())
        );
    }

    #[test]
    fn bing_plain_href_passes_through() {
        assert_eq!(
            resolve_bing_href("https://example.com/x"),
            Some("https://example.com/x".to_string())
        );
        assert_eq!(resolve_bing_href("/relative"), None);
    }

    #[test]
    fn bing_block_parses_title_url_snippet() {
        let html = r#"<ol id="b_results">
<li class="b_algo" data-id><link rel="stylesheet" href="https://r.bing.com/x.css"/>
<div class="b_tpcn"><a class="tilk" href="https://www.bing.com/ck/a?u=a1aHR0cHM6Ly92Mi50YXVyaS5vcmcuY24v"><span>tauri.org.cn</span>https://v2.tauri.org.cn</a></div>
<h2><a href="https://www.bing.com/ck/a?u=a1aHR0cHM6Ly92Mi50YXVyaS5vcmcuY24v">Tauri 官网</a></h2>
<p class="b_lineclamp2">Build smaller, faster apps.</p></li>"#;
        let results = parse_bing(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://v2.tauri.org.cn/");
        // 标题取 <h2>，不带域名面包屑。
        assert_eq!(results[0].title, "Tauri 官网");
        assert_eq!(results[0].snippet.as_deref(), Some("Build smaller, faster apps."));
    }

    #[test]
    fn ddg_uddg_redirect_is_unwrapped() {
        let line = r#"<a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&amp;rut=abc" class="result-link">Example</a>"#;
        assert_eq!(
            extract_ddg_href(line),
            Some("https://example.com/page".to_string())
        );
    }

    #[test]
    fn ddg_internal_links_are_skipped() {
        assert_eq!(extract_ddg_href(r#"<a href="/lite/">Home</a>"#), None);
        assert_eq!(
            extract_ddg_href(r#"<a href="//duckduckgo.com/about">About</a>"#),
            None
        );
    }

    #[test]
    fn domain_filters_and_dedup_apply() {
        let results = vec![
            SearchResult { title: "a".into(), url: "https://rust-lang.org/x".into(), snippet: None },
            SearchResult { title: "a".into(), url: "https://rust-lang.org/x".into(), snippet: None },
            SearchResult { title: "b".into(), url: "https://spam.com/y".into(), snippet: None },
        ];
        let filtered = filter_results(results, &[], &["spam.com".into()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].url, "https://rust-lang.org/x");

        let results = vec![
            SearchResult { title: "a".into(), url: "https://docs.rs/x".into(), snippet: None },
            SearchResult { title: "b".into(), url: "https://other.com/y".into(), snippet: None },
        ];
        let filtered = filter_results(results, &["*.rs".into()], &[]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].url, "https://docs.rs/x");
    }

    #[test]
    fn truncate_helpers_are_multibyte_safe() {
        let cjk = "中文内容测试";
        assert!(truncate_utf8(cjk, 7).chars().count() <= 2);
        assert_eq!(truncate_chars(cjk, 3), "中文内…");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }
}
