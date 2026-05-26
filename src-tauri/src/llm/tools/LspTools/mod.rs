use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![
        app_tool(status_tool, status_with_app, true, None),
        app_tool(diagnostics_tool, diagnostics_with_app, true, None),
        app_tool(definition_tool, definition_with_app, true, None),
        app_tool(references_tool, references_with_app, true, None),
        app_tool(symbols_tool, symbols_with_app, true, None),
        app_tool(hover_tool, hover_with_app, true, None),
    ]
}

fn string_arg(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_string(input: &Value, tool_name: &str, key: &str) -> Result<String, String> {
    string_arg(input, key).ok_or_else(|| format!("{} requires {}", tool_name, key))
}

fn required_u64(input: &Value, tool_name: &str, key: &str) -> Result<u64, String> {
    input
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{} requires 1-based {}", tool_name, key))
}

fn result_json<T: serde::Serialize>(result: Result<T, String>) -> Result<ToolOutcome, ToolFailure> {
    match result {
        Ok(result) => Ok(ToolOutcome::json(json!({ "ok": true, "result": result }))),
        Err(error) => Err(ToolFailure::new(error)),
    }
}

fn status_tool() -> Tool {
    Tool {
        name: "lsp_status".into(),
        description: "Show native language-server availability, running state, and cached diagnostic counts for the current workspace.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn status_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    _input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        result_json(crate::llm::services::lsp::status(&app, conversation_id.as_deref()).await)
    })
}

fn diagnostics_tool() -> Tool {
    Tool {
        name: "lsp_diagnostics".into(),
        description: "Read native LSP diagnostics for a workspace file. If path is omitted, returns cached diagnostics from running language servers.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative or absolute path to a source file"
                }
            }
        }),
    }
}

fn diagnostics_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        result_json(
            crate::llm::services::lsp::diagnostics(
                &app,
                conversation_id.as_deref(),
                string_arg(&input, "path"),
            )
            .await,
        )
    })
}

fn definition_tool() -> Tool {
    Tool {
        name: "lsp_definition".into(),
        description:
            "Find native LSP definition locations for a symbol at a 1-based file position.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative or absolute source file path" },
                "line": { "type": "integer", "description": "1-based line number" },
                "character": { "type": "integer", "description": "1-based character/column number" }
            },
            "required": ["path", "line", "character"]
        }),
    }
}

fn definition_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let result = async {
            crate::llm::services::lsp::definition(
                &app,
                conversation_id.as_deref(),
                required_string(&input, "lsp_definition", "path")?,
                required_u64(&input, "lsp_definition", "line")?,
                required_u64(&input, "lsp_definition", "character")?,
            )
            .await
        }
        .await;
        result_json(result)
    })
}

fn references_tool() -> Tool {
    Tool {
        name: "lsp_references".into(),
        description: "Find native LSP references for a symbol at a 1-based file position.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative or absolute source file path" },
                "line": { "type": "integer", "description": "1-based line number" },
                "character": { "type": "integer", "description": "1-based character/column number" },
                "includeDeclaration": { "type": "boolean", "description": "Whether to include the declaration location" }
            },
            "required": ["path", "line", "character"]
        }),
    }
}

fn references_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let result = async {
            crate::llm::services::lsp::references(
                &app,
                conversation_id.as_deref(),
                required_string(&input, "lsp_references", "path")?,
                required_u64(&input, "lsp_references", "line")?,
                required_u64(&input, "lsp_references", "character")?,
                input
                    .get("includeDeclaration")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            )
            .await
        }
        .await;
        result_json(result)
    })
}

fn symbols_tool() -> Tool {
    Tool {
        name: "lsp_symbols".into(),
        description: "Read native LSP document symbols for a file, or workspace symbols when query is provided without path.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Optional workspace-relative or absolute source file path" },
                "query": { "type": "string", "description": "Optional workspace symbol query" }
            }
        }),
    }
}

fn symbols_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        result_json(
            crate::llm::services::lsp::symbols(
                &app,
                conversation_id.as_deref(),
                string_arg(&input, "path"),
                input
                    .get("query")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
            .await,
        )
    })
}

fn hover_tool() -> Tool {
    Tool {
        name: "lsp_hover".into(),
        description: "Read native LSP hover information at a 1-based file position.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Workspace-relative or absolute source file path" },
                "line": { "type": "integer", "description": "1-based line number" },
                "character": { "type": "integer", "description": "1-based character/column number" }
            },
            "required": ["path", "line", "character"]
        }),
    }
}

fn hover_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let result = async {
            crate::llm::services::lsp::hover(
                &app,
                conversation_id.as_deref(),
                required_string(&input, "lsp_hover", "path")?,
                required_u64(&input, "lsp_hover", "line")?,
                required_u64(&input, "lsp_hover", "character")?,
            )
            .await
        }
        .await;
        result_json(result)
    })
}
