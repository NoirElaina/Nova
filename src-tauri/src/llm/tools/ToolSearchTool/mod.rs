use crate::llm::services::mcp_tools;
use crate::llm::tools::{app_tool, get_available_tools, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::AppHandle;

// 注册 tool_search，声明它是只读 app 工具，可以在运行时把 MCP 动态工具也并进搜索结果。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

// 返回暴露给模型的工具元数据，要求传入 query 作为搜索关键字。
pub fn tool() -> Tool {
    Tool {
        name: "tool_search".into(),
        description: "Search available tool names by keyword.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
    }
}

// 搜索内置工具加已连接的 MCP 动态工具；这是运行时实际使用的完整搜索路径。
pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'query' argument" }).to_string(),
    };

    let mut tools = get_available_tools();
    tools.extend(mcp_tools::collect_mcp_tools(app).await);
    search_tools(query, tools)
}

// 把 async execute_with_app 包成统一的 AppExecuteFuture，供注册层调用。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// query 是搜索关键字；tools 是待搜索的工具列表；返回按名字排序后的匹配结果文本。
fn search_tools(query: &str, tools: Vec<Tool>) -> String {
    let query = query.trim();
    // match_all: query 为 "*" 时直接返回全部工具。
    let match_all = query == "*";
    // normalized_query: 统一转小写，避免大小写差异影响匹配。
    let normalized_query = query.to_ascii_lowercase();
    // seen: 用来去重，避免内置工具与 MCP 工具重名时重复输出。
    let mut seen = HashSet::new();
    let mut matched: Vec<String> = tools
        .into_iter()
        .filter(|tool| {
            if match_all {
                return true;
            }
            let searchable = format!(
                "{} {}",
                tool.name.to_ascii_lowercase(),
                tool.description.to_ascii_lowercase()
            );
            searchable.contains(&normalized_query)
        })
        .filter(|tool| seen.insert(tool.name.clone()))
        .map(|tool| format!("{}: {}", tool.name, tool.description))
        .collect();
    matched.sort();

    if matched.is_empty() {
        "No matching tools found".into()
    } else {
        matched.join("\n")
    }
}
