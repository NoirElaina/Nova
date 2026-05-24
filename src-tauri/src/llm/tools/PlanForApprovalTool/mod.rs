use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 返回 plan_for_approval 的注册信息。
// 这个工具会暂停执行并等待用户审批计划，因此不是只读工具。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 从 input[key] 里读取一个非空字符串。
// 这里统一做 trim，避免后面每个字段都重复写空白处理逻辑。
fn normalized_non_empty_string(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// 从 input[key] 里读取字符串数组，并过滤掉空字符串项。
// 这个函数主要给 `steps` 和 `risks` 复用。
fn normalized_string_list(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

// 把字符串列表格式化成 `1. xxx` 这种编号段落。
// `items` 一般是计划步骤或风险点。
fn join_bullets(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| format!("{}. {}", idx + 1, item))
        .collect::<Vec<_>>()
        .join("\n")
}

// 返回模型可见的 plan_for_approval 元数据。
// 模型需要提供 `summary` 和 `steps`，工具再把它们包装成一个审批问题。
pub fn tool() -> Tool {
    Tool {
        name: "plan_for_approval".into(),
        description: "Present an implementation plan to the user and request explicit approval before making code changes.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short plan title"
                },
                "summary": {
                    "type": "string",
                    "description": "High-level plan summary"
                },
                "steps": {
                    "type": "array",
                    "description": "Concrete implementation steps",
                    "items": { "type": "string" }
                },
                "risks": {
                    "type": "array",
                    "description": "Optional risks/tradeoffs",
                    "items": { "type": "string" }
                },
                "allow_freeform": {
                    "type": "boolean",
                    "description": "Whether user can provide freeform adjustments"
                }
            },
            "required": ["summary", "steps"]
        }),
    }
}

// 把计划摘要、步骤和风险整理成 `needs_user_input` payload，请用户明确审批。
// `summary` 是高层概述，`steps` 是具体实施步骤，`risks` 是可选风险提醒。
fn execute_local(input: Value) -> String {
    let summary = match normalized_non_empty_string(&input, "summary") {
        Some(v) => v,
        None => {
            return json!({ "ok": false, "error": "Missing or empty 'summary' argument" })
                .to_string()
        }
    };

    let steps = normalized_string_list(&input, "steps");
    if steps.is_empty() {
        return json!({ "ok": false, "error": "Missing non-empty 'steps' array" }).to_string();
    }

    // title: 展示给用户看的计划标题，不传时使用默认“计划提审”。
    let title = normalized_non_empty_string(&input, "title")
        .unwrap_or_else(|| "计划提审".to_string());
    let risks = normalized_string_list(&input, "risks");
    let allow_freeform = input
        .get("allow_freeform")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // context_lines: 拼成审批弹窗正文的多行文本，前端最终会把它展示给用户。
    let mut context_lines = vec![
        format!("{}：", title),
        format!("摘要：{}", summary),
        "".to_string(),
        "实施步骤：".to_string(),
        join_bullets(&steps),
    ];

    if !risks.is_empty() {
        context_lines.push("".to_string());
        context_lines.push("风险与注意点：".to_string());
        context_lines.push(join_bullets(&risks));
    }

    json!({
        "type": "needs_user_input",
        "context": context_lines.join("\n"),
        "questions": [
            {
                "header": "计划确认",
                "question": "是否批准按此计划实施？",
                "multi_select": false,
                "options": [
                    {
                        "label": "批准并开始实施",
                        "description": "计划确认无误，进入实现阶段",
                        "value": "approve_and_implement"
                    },
                    {
                        "label": "先修改计划",
                        "description": "先根据你的反馈调整计划，再次提审",
                        "value": "revise_plan"
                    },
                    {
                        "label": "暂不实施",
                        "description": "保留当前计划，不进入代码修改",
                        "value": "hold"
                    }
                ]
            }
        ],
        "allow_freeform": allow_freeform,
        "instruction": "Stop execution and wait for user decision before implementation."
    })
    .to_string()
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_local(input) })
}
