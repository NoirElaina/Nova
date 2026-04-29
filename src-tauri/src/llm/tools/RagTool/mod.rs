use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "rag_tool".into(),
        description: "Search and read documents from Nova local RAG database.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["stats", "search", "read"]
                },
                "query": { "type": "string" },
                "documentId": { "type": "string" },
                "id": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["action"]
        }),
    }
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "rag_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "stats" => match crate::command::rag::rag_get_stats(app.clone()) {
            Ok(stats) => json!({
                "ok": true,
                "action": "stats",
                "stats": stats
            })
            .to_string(),
            Err(e) => json!({ "ok": false, "error": e }).to_string(),
        },
        "search" => {
            let Some(query) = input
                .get("query")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return json!({
                    "ok": false,
                    "error": "rag_tool search requires non-empty 'query'"
                })
                .to_string();
            };

            let limit = input
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            match crate::command::rag::rag_search_documents(app.clone(), query.to_string(), limit) {
                Ok(results) => json!({
                    "ok": true,
                    "action": "search",
                    "query": query,
                    "results": results
                })
                .to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        "read" => {
            let Some(document_id) = input
                .get("documentId")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return json!({
                    "ok": false,
                    "error": "rag_tool read requires non-empty 'documentId' or 'id'"
                })
                .to_string();
            };

            match crate::command::rag::rag_read_document(app.clone(), document_id.to_string()) {
                Ok(Some(document)) => json!({
                    "ok": true,
                    "action": "read",
                    "document": document
                })
                .to_string(),
                Ok(None) => json!({
                    "ok": true,
                    "action": "read",
                    "retrieval_status": "not_found",
                    "document": Value::Null
                })
                .to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        _ => json!({
            "ok": false,
            "error": "rag_tool action must be one of: stats, search, read"
        })
        .to_string(),
    }
}
