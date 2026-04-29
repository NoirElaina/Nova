use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "lsp_tool".into(),
        description: "Run semantic code-navigation operations via MCP-backed LSP servers (list servers/tools, call, find symbol/references/definition/implementation, diagnostics).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list_servers",
                        "list_server_tools",
                        "call",
                        "find_symbol",
                        "find_references",
                        "find_definition",
                        "find_implementation",
                        "diagnostics"
                    ]
                },
                "server": { "type": "string" },
                "tool": { "type": "string" },
                "symbol": { "type": "string" },
                "file": { "type": "string" },
                "lineContent": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["action"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("unknown");
    json!({
        "ok": false,
        "action": action,
        "message": "lsp_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

fn lsp_keywords_for_action(action: &str) -> &'static [&'static str] {
    match action {
        "find_symbol" => &["symbol", "workspace", "document_symbol", "symbols"],
        "find_references" => &["reference", "references", "usage", "usages"],
        "find_definition" => &["definition", "goto_definition", "definitions"],
        "find_implementation" => &["implementation", "implementations"],
        "diagnostics" => &["diagnostic", "diagnostics", "problem", "problems", "error", "errors"],
        _ => &[],
    }
}

fn is_lsp_candidate_tool_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "lsp",
        "symbol",
        "reference",
        "definition",
        "implementation",
        "diagnostic",
        "workspace",
        "document",
        "hover",
        "rename",
        "usage",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

fn choose_lsp_tool_name(action: &str, tools: &[crate::command::mcp::McpToolInfo]) -> Option<String> {
    let keywords = lsp_keywords_for_action(action);
    if keywords.is_empty() {
        return None;
    }

    tools
        .iter()
        .find(|tool| {
            let name = tool.name.to_ascii_lowercase();
            keywords.iter().any(|kw| name.contains(kw))
        })
        .map(|tool| tool.name.clone())
}

fn merge_lsp_arguments(input: &Value) -> Value {
    let mut map = input
        .get("arguments")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    if !map.contains_key("symbol") {
        if let Some(symbol) = input
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            map.insert("symbol".to_string(), Value::String(symbol.to_string()));
        }
    }

    if !map.contains_key("file") {
        if let Some(file) = input
            .get("file")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            map.insert("file".to_string(), Value::String(file.to_string()));
        }
    }

    if !map.contains_key("lineContent") {
        if let Some(line_content) = input
            .get("lineContent")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            map.insert(
                "lineContent".to_string(),
                Value::String(line_content.to_string()),
            );
        }
    }

    Value::Object(map)
}

pub async fn execute_with_app(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> String {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "list_servers" => {
            let statuses = match crate::command::mcp::get_mcp_server_statuses(app.clone()).await {
                Ok(v) => v,
                Err(e) => {
                    return json!({ "ok": false, "error": e }).to_string();
                }
            };

            let mut rows = Vec::new();
            for status in statuses {
                let lsp_tools = if status.status == "connected" {
                    match crate::command::mcp::list_mcp_tools(app.clone(), status.name.clone()).await {
                        Ok(tools) => tools
                            .into_iter()
                            .map(|t| t.name)
                            .filter(|name| is_lsp_candidate_tool_name(name))
                            .collect::<Vec<_>>(),
                        Err(_) => Vec::new(),
                    }
                } else {
                    Vec::new()
                };

                rows.push(json!({
                    "name": status.name,
                    "status": status.status,
                    "enabled": status.enabled,
                    "type": status.r#type,
                    "toolCount": status.tool_count,
                    "error": status.error,
                    "lspToolCount": lsp_tools.len(),
                    "lspTools": lsp_tools,
                }));
            }

            json!({
                "ok": true,
                "action": "list_servers",
                "servers": rows
            })
            .to_string()
        }
        "list_server_tools" => {
            let Some(server_name) = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return json!({
                    "ok": false,
                    "error": "lsp_tool list_server_tools requires non-empty 'server'"
                })
                .to_string();
            };

            match crate::command::mcp::list_mcp_tools(app.clone(), server_name.to_string()).await {
                Ok(tools) => {
                    let lsp_tools = tools
                        .iter()
                        .map(|t| t.name.clone())
                        .filter(|name| is_lsp_candidate_tool_name(name))
                        .collect::<Vec<_>>();

                    json!({
                        "ok": true,
                        "action": "list_server_tools",
                        "server": server_name,
                        "tools": tools,
                        "lspTools": lsp_tools,
                    })
                    .to_string()
                }
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        "call" | "find_symbol" | "find_references" | "find_definition" | "find_implementation" | "diagnostics" => {
            let explicit_server = input
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            let target_server = if let Some(server) = explicit_server {
                server
            } else {
                let statuses = match crate::command::mcp::get_mcp_server_statuses(app.clone()).await {
                    Ok(v) => v,
                    Err(e) => {
                        return json!({ "ok": false, "error": e }).to_string();
                    }
                };

                let mut chosen = None;
                for status in statuses
                    .into_iter()
                    .filter(|s| s.enabled && s.status == "connected")
                {
                    if let Ok(tools) =
                        crate::command::mcp::list_mcp_tools(app.clone(), status.name.clone()).await
                    {
                        if tools.iter().any(|t| is_lsp_candidate_tool_name(&t.name)) {
                            chosen = Some(status.name);
                            break;
                        }
                    }
                }

                let Some(server) = chosen else {
                    return json!({
                        "ok": false,
                        "error": "No connected MCP server exposing LSP-like tools; set 'server' explicitly or connect an LSP MCP server"
                    })
                    .to_string();
                };
                server
            };

            let available_tools =
                match crate::command::mcp::list_mcp_tools(app.clone(), target_server.clone()).await {
                    Ok(v) => v,
                    Err(e) => {
                        return json!({ "ok": false, "error": e }).to_string();
                    }
                };

            let explicit_tool = input
                .get("tool")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            let target_tool = if let Some(tool_name) = explicit_tool {
                tool_name
            } else if action == "call" {
                return json!({
                    "ok": false,
                    "error": "lsp_tool call requires non-empty 'tool'"
                })
                .to_string();
            } else {
                let Some(tool_name) = choose_lsp_tool_name(&action, &available_tools) else {
                    let names = available_tools.into_iter().map(|t| t.name).collect::<Vec<_>>();
                    return json!({
                        "ok": false,
                        "error": format!("No suitable LSP tool found for action '{}' on server '{}'", action, target_server),
                        "availableTools": names
                    })
                    .to_string();
                };
                tool_name
            };

            let call_output =
                crate::llm::tools::shared::permission_runtime::call_mcp_tool_with_nested_permission(
                    app,
                    conversation_id,
                    target_server,
                    target_tool,
                    merge_lsp_arguments(&input),
                )
                .await;

            if crate::llm::tools::shared::permission_runtime::is_needs_user_input_payload(&call_output)
            {
                call_output
            } else {
                let parsed = serde_json::from_str::<Value>(&call_output)
                    .unwrap_or_else(|_| Value::String(call_output.clone()));
                json!({
                    "ok": true,
                    "action": action,
                    "result": parsed
                })
                .to_string()
            }
        }
        _ => json!({
            "ok": false,
            "error": "lsp_tool action must be one of: list_servers, list_server_tools, call, find_symbol, find_references, find_definition, find_implementation, diagnostics"
        })
        .to_string(),
    }
}
