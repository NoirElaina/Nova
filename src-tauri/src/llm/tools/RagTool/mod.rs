use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把 RAG 工具的 async 执行逻辑包装成统一 future。
// `input` 里会带 action/query/document_id/limit 这些查询参数。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 rag_tool 的注册信息。
// RAG 只做本地知识库读取，因此标成只读工具。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true, None)
}

// 返回模型可见的 rag_tool 元数据。
// `action` 决定本次是查统计、搜索文档，还是读取单篇文档内容。
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
                "document_id": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["action"]
        }),
    }
}

// 同步入口只返回提示，要求调用方改走带 AppHandle 的 RAG 执行路径。
pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "rag_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

// 根据 `action` 访问本地 RAG 数据库。
// `query` 只在 search 分支使用，`document_id` 只在 read 分支使用。
pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    // action: 统一转成小写后的操作类型，避免模型大小写混用时匹配失败。
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

            // limit: 搜索结果数量上限；不传时交给底层 RAG 默认值处理。
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
                .get("document_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return json!({
                    "ok": false,
                    "error": "rag_tool read requires non-empty 'document_id'"
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
