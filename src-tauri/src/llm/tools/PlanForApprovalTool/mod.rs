use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

fn normalized_non_empty_string(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

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

fn join_bullets(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| format!("{}. {}", idx + 1, item))
        .collect::<Vec<_>>()
        .join("\n")
}

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

pub fn execute(input: Value) -> String {
    let summary = match normalized_non_empty_string(&input, "summary") {
        Some(v) => v,
        None => return "Error: Missing or empty 'summary' argument".into(),
    };

    let steps = normalized_string_list(&input, "steps");
    if steps.is_empty() {
        return "Error: Missing non-empty 'steps' array".into();
    }

    let title = normalized_non_empty_string(&input, "title")
        .unwrap_or_else(|| "计划提审".to_string());
    let risks = normalized_string_list(&input, "risks");
    let allow_freeform = input
        .get("allow_freeform")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

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
