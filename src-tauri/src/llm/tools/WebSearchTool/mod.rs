use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;
use url::Url;

// 返回 web_search 的注册信息。
// 这个工具只生成搜索链接，所以可以作为只读工具并发执行。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回模型可见的 web_search 元数据。
// 模型传入 `query` 后，这个工具不会联网搜索，只负责构造后续可抓取的搜索结果页 URL。
pub fn tool() -> Tool {
    Tool {
        name: "web_search".into(),
        description: "Create a web search URL for a query and provide guidance for next fetch.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" }
            },
            "required": ["query"]
        }),
    }
}

fn search_url(base: &str, query: &str) -> Result<String, String> {
    Url::parse_with_params(base, &[("q", query)])
        .map(|url| url.to_string())
        .map_err(|e| format!("Failed to build search URL: {e}"))
}

// 根据 `query` 生成搜索引擎 URL，供后续 `web_fetch` 继续抓取。
// URL 编码交给 `url` crate，避免中文、特殊字符或空格被错误拼接。
fn execute_local(input: Value) -> String {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'query' argument" }).to_string(),
    };

    let duckduckgo = match search_url("https://duckduckgo.com/", query) {
        Ok(url) => url,
        Err(e) => return json!({ "ok": false, "error": e }).to_string(),
    };
    let bing = match search_url("https://www.bing.com/search", query) {
        Ok(url) => url,
        Err(e) => return json!({ "ok": false, "error": e }).to_string(),
    };

    json!({
        "ok": true,
        "query": query,
        "search_urls": {
            "duckduckgo": duckduckgo,
            "bing": bing
        },
        "note": "Use web_fetch with one of these URLs to inspect result pages."
    })
    .to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_local(input) })
}
