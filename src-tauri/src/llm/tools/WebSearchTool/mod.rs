use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

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

pub fn execute(input: Value) -> String {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return "Error: Missing 'query' argument".into(),
    };

    let encoded = query.replace(' ', "+");
    let ddg = format!("https://duckduckgo.com/?q={}", encoded);
    let bing = format!("https://www.bing.com/search?q={}", encoded);

    json!({
        "query": query,
        "search_urls": {
            "duckduckgo": ddg,
            "bing": bing
        },
        "note": "Use web_fetch with one of these URLs to inspect result pages."
    })
    .to_string()
}
