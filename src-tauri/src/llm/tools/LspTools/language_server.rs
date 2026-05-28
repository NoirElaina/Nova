use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use crate::llm::utils::paths::absolute_path_from_tool_arg;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registrations() -> Vec<ToolRegistration> {
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

fn absolute_path_string(path: String, tool_name: &str, key: &str) -> Result<String, String> {
    absolute_path_from_tool_arg(&path, key)
        .map(|path| path.display().to_string())
        .map_err(|error| format!("{} {}", tool_name, error))
}

fn required_absolute_path(input: &Value, tool_name: &str, key: &str) -> Result<String, String> {
    absolute_path_string(required_string(input, tool_name, key)?, tool_name, key)
}

fn optional_absolute_path(
    input: &Value,
    tool_name: &str,
    key: &str,
) -> Result<Option<String>, String> {
    string_arg(input, key)
        .map(|path| absolute_path_string(path, tool_name, key).map(Some))
        .unwrap_or(Ok(None))
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
        description: "Read native LSP diagnostics for an absolute file path. If path is omitted, returns cached diagnostics from running language servers.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to a source file. Relative paths and ~ are rejected."
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
        let result = async {
            let path = optional_absolute_path(&input, "lsp_diagnostics", "path")?;
            crate::llm::services::lsp::diagnostics(&app, conversation_id.as_deref(), path).await
        }
        .await;
        result_json(result)
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
                "path": { "type": "string", "description": "Absolute source file path. Relative paths and ~ are rejected." },
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
                required_absolute_path(&input, "lsp_definition", "path")?,
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
                "path": { "type": "string", "description": "Absolute source file path. Relative paths and ~ are rejected." },
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
                required_absolute_path(&input, "lsp_references", "path")?,
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
                "path": { "type": "string", "description": "Optional absolute source file path. Relative paths and ~ are rejected." },
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
        let result = async {
            let path = optional_absolute_path(&input, "lsp_symbols", "path")?;
            crate::llm::services::lsp::symbols(
                &app,
                conversation_id.as_deref(),
                path,
                input
                    .get("query")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
            .await
        }
        .await;
        result_json(result)
    })
}

fn hover_tool() -> Tool {
    Tool {
        name: "lsp_hover".into(),
        description: "Read native LSP hover information at a 1-based file position.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute source file path. Relative paths and ~ are rejected." },
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
                required_absolute_path(&input, "lsp_hover", "path")?,
                required_u64(&input, "lsp_hover", "line")?,
                required_u64(&input, "lsp_hover", "character")?,
            )
            .await
        }
        .await;
        result_json(result)
    })
}
