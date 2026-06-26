use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把 RAG 工具的 async 执行逻辑包装成统一 future。
// `input` 里会带 action/query/document_id/limit 这些查询参数。
fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id, input).await })
}

// 返回 rag_tool 的注册信息。
// RAG 只做本地知识库读取，因此标成只读工具。
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
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
                    "enum": ["stats", "search", "read", "fetch"]
                },
                "query": { "type": "string", "description": "Used by 'search': keyword query terms." },
                "document_id": { "type": "string", "description": "Used by 'read': group_id returned by search." },
                "source_name": { "type": "string", "description": "Used by 'fetch': exact file name. Returns full document text in one step." },
                "limit": { "type": "integer" }
            },
            "required": ["action"]
        }),
    }
}

// 根据 `action` 访问本地 RAG 数据库。
// `query` 只在 search 分支使用，`document_id` 只在 read 分支使用。
async fn execute_with_app(
    app: &AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    // action: 统一转成小写后的操作类型，避免模型大小写混用时匹配失败。
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "stats" => match crate::command::rag::rag_get_stats(app.clone()).await {
            Ok(stats) => Ok(ToolOutcome::json(json!({
                "ok": true,
                "action": "stats",
                "stats": stats
            }))),
            Err(e) => Err(ToolFailure::new(e)),
        },
        "search" => {
            let Some(query) = input
                .get("query")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Err(ToolFailure::invalid_input(
                    "rag_tool search requires non-empty 'query'",
                ));
            };

            let limit = input
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let result =
                crate::command::rag::rag_search_documents(app.clone(), query.to_string(), limit)
                    .await;

            match result {
                Ok(results) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "search",
                    "query": query,
                    "results": results
                }))),
                Err(e) => Err(ToolFailure::new(e)),
            }
        }
        "read" => {
            let Some(document_id) = input
                .get("document_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Err(ToolFailure::invalid_input(
                    "rag_tool read requires non-empty 'document_id'",
                ));
            };

            match crate::command::rag::rag_read_document(
                app.clone(),
                document_id.to_string(),
            )
            .await
            {
                Ok(Some(document)) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "read",
                    "document": document
                }))),
                Ok(None) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "read",
                    "retrieval_status": "not_found",
                    "document": Value::Null
                }))),
                Err(e) => Err(ToolFailure::new(e)),
            }
        }
        "fetch" => {
            let Some(source_name) = input
                .get("source_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Err(ToolFailure::invalid_input(
                    "rag_tool fetch requires non-empty 'source_name'",
                ));
            };

            let docs = crate::command::rag::rag_list_documents(app.clone()).await;

            let docs = match docs {
                Ok(d) => d,
                Err(e) => return Err(ToolFailure::new(e)),
            };

            let Some(meta) = docs.iter().find(|d| d.source_name == source_name) else {
                return Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "fetch",
                    "retrieval_status": "not_found",
                    "document": Value::Null
                })));
            };

            match crate::command::rag::rag_read_document(
                app.clone(),
                meta.id.clone(),
            )
            .await
            {
                Ok(Some(document)) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "fetch",
                    "document": document
                }))),
                Ok(None) => Ok(ToolOutcome::json(json!({
                    "ok": true,
                    "action": "fetch",
                    "retrieval_status": "not_found",
                    "document": Value::Null
                }))),
                Err(e) => Err(ToolFailure::new(e)),
            }
        }
        _ => Err(ToolFailure::invalid_input(
            "rag_tool action must be one of: stats, search, read, fetch",
        )),
    }
}
