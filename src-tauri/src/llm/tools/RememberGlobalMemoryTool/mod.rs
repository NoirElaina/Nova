use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 把全局记忆写入逻辑包装成统一 future。
// `input` 里主要读取 content/kind/source 三个字段。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 remember_global_memory 的注册信息。
// 这是写类工具，因为它会把记忆持久化到跨会话存储中。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回模型可见的 remember_global_memory 元数据。
// 模型至少要提供 `content`，可选再补 `kind` 和 `source`。
pub fn tool() -> Tool {
    Tool {
        name: "remember_global_memory".into(),
        description: "Persist a stable cross-session memory item (preference/fact/rule) without user confirmation.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Memory content to persist globally"
                },
                "kind": {
                    "type": "string",
                    "enum": ["preference", "fact", "rule"],
                    "description": "Memory type classification"
                },
                "source": {
                    "type": "string",
                    "description": "Optional source tag, default assistant"
                }
            },
            "required": ["content"]
        }),
    }
}

// 把一条稳定信息写进全局记忆库。
// `content` 是真正要保存的内容，`kind` 是分类标签，`source` 标记来源。
pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let content = match input.get("content").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => {
            return json!({ "ok": false, "error": "Missing non-empty 'content' argument" })
                .to_string()
        }
    };

    // kind/source: 这两个字段都是可选元数据，会原样传给底层 memory 写入逻辑。
    let kind = input.get("kind").and_then(|v| v.as_str());
    let source = input.get("source").and_then(|v| v.as_str());

    match crate::llm::history::upsert_global_memory(app, content, kind, source).await {
        Ok(entry) => json!({ "ok": true, "memory": entry }).to_string(),
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    }
}
