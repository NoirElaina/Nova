use crate::llm::services::tool_disclosure;
use crate::llm::tools::{
    app_tool, deferred_tool_definitions, AppExecuteFuture, ToolDisclosure, ToolFailure,
    ToolOutcome, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    // LoadTool 是披露机制的入口，自身必须始终可见（Core）、只读、免审批。
    app_tool(tool, execute_with_app_boxed, true, None, ToolDisclosure::Core)
}

pub fn tool() -> Tool {
    Tool {
        name: "LoadTool".into(),
        description: "Load on-demand tools that are not in the default tool list. \
Pass exact tool names via `names`, or a keyword `query` to discover matching tools. \
Loaded tools become callable from the next step and stay available for this conversation."
            .into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Exact tool names to load (case-insensitive), e.g. [\"WebSearch\", \"CronCreate\"]."
                },
                "query": {
                    "type": "string",
                    "description": "Keyword search over available on-demand tools; loads every match."
                }
            }
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id, input).await })
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn query_matches(haystack: &str, query: &str) -> bool {
    let haystack = haystack.to_ascii_lowercase();
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return false;
    }
    // 全部关键字都命中（按空白切分）才算匹配。
    query
        .split_whitespace()
        .all(|token| haystack.contains(token))
}

async fn execute_with_app(
    app: &AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let names: Vec<String> = input
        .get("names")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    if names.is_empty() && query.is_empty() {
        return Err(ToolFailure::invalid_input(
            "LoadTool requires 'names' and/or 'query'",
        ));
    }

    let catalog = deferred_tool_definitions();

    // 收集要加载的工具名：精确名字 + 关键字命中。
    let mut wanted: Vec<String> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for name in &names {
        let normalized = normalize_name(name);
        if catalog
            .iter()
            .any(|tool| normalize_name(&tool.name) == normalized)
        {
            wanted.push(
                catalog
                    .iter()
                    .find(|tool| normalize_name(&tool.name) == normalized)
                    .map(|tool| tool.name.clone())
                    .unwrap_or_default(),
            );
        } else {
            not_found.push(name.clone());
        }
    }

    if !query.is_empty() {
        for tool in &catalog {
            let haystack = format!("{} {}", tool.name, tool.description);
            if query_matches(&haystack, &query) && !wanted.contains(&tool.name) {
                wanted.push(tool.name.clone());
            }
        }
    }

    if wanted.is_empty() {
        let available: Vec<&str> = catalog.iter().map(|tool| tool.name.as_str()).collect();
        return Err(ToolFailure::invalid_input(format!(
            "No on-demand tool matched (not found: {}). Available on-demand tools: {}",
            not_found.join(", "),
            available.join(", ")
        )));
    }

    let newly_loaded = tool_disclosure::mark_disclosed(
        app,
        conversation_id.as_deref(),
        &wanted,
    );

    // 回传完整定义（含 schema），模型下一步即可按定义调用。
    let loaded: Vec<Value> = catalog
        .iter()
        .filter(|tool| wanted.contains(&tool.name))
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "loaded": loaded,
        "newly_loaded": newly_loaded,
        "not_found": not_found,
        "note": "Loaded tools are callable starting from your next step.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_requires_all_tokens() {
        assert!(query_matches("WebSearch online search engine", "web search"));
        assert!(query_matches("WebSearch online search engine", "SEARCH"));
        assert!(!query_matches("WebSearch online search engine", "web cron"));
        assert!(!query_matches("WebSearch", ""));
    }

    #[test]
    fn name_normalization() {
        assert_eq!(normalize_name(" WebSearch "), "websearch");
    }
}
