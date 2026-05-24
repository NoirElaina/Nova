use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

// 返回 ask_user_question 的注册信息。
// 这个工具会中断当前执行流并等待用户回答，所以不是只读工具。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, false, None)
}

// 返回 ask_user_question 暴露给模型的元数据。
// 当前接口只接受 `questions[]`，即使只问一个问题也要放进数组里。
pub fn tool() -> Tool {
    Tool {
        name: "ask_user_question".into(),
        description: "Ask the user one to four structured clarifying questions when required information is missing before continuing.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 4,
                    "description": "One to four short questions to ask the user",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "The exact question to ask the user"
                            },
                            "header": {
                                "type": "string",
                                "description": "A short label shown above the question"
                            },
                            "multi_select": {
                                "type": "boolean",
                                "description": "Whether the user can select multiple options for this question"
                            },
                            "options": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 4,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "Short option label"
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "What this option means"
                                        },
                                        "preview": {
                                            "type": "string",
                                            "description": "Optional preview or mockup text for the option"
                                        }
                                    },
                                    "required": ["label", "description"]
                                }
                            }
                        },
                        "required": ["question", "header", "options"]
                    }
                },
                "context": {
                    "type": "string",
                    "description": "Short reason why these questions are needed"
                },
                "allow_freeform": {
                    "type": "boolean",
                    "description": "Whether user can answer outside provided options"
                }
            },
            "required": ["questions"]
        }),
    }
}

// 规范化单个选项对象。
// `option` 里至少要有 `label` 和 `description`，`preview` 是可选的预览文本。
fn normalize_question_option(option: &Value) -> Option<Value> {
    let label = option.get("label").and_then(|v| v.as_str())?.trim().to_string();
    let description = option
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let preview = option
        .get("preview")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if label.is_empty() || description.is_empty() {
        return None;
    }

    Some(json!({
        "label": label,
        "description": description,
        "preview": preview
    }))
}

// 规范化单个问题对象。
// `question` 里会读取 `question`/`header`/`options`/`multi_select`，不合法时直接丢弃。
fn normalize_question(question: &Value) -> Option<Value> {
    let question_text = question
        .get("question")
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    let header = question
        .get("header")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let options = question
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(normalize_question_option)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let multi_select = question
        .get("multi_select")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if question_text.is_empty() || header.is_empty() || options.len() < 2 {
        return None;
    }

    Some(json!({
        "question": question_text,
        "header": header,
        "multi_select": multi_select,
        "options": options
    }))
}

// 把模型传来的提问参数整理成统一的 `needs_user_input` payload。
// `context` 是为什么要提问，`questions` 是最终展示给用户的问题数组。
fn execute_local(input: Value) -> String {
    let context = input
        .get("context")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // allow_freeform: true 时，前端允许用户输入选项之外的自由文本回答。
    let allow_freeform = input
        .get("allow_freeform")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // questions: 当前接口要求模型直接传标准 questions[] 数组。
    let questions = input
        .get("questions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(normalize_question).collect::<Vec<_>>())
        .unwrap_or_default();

    if questions.is_empty() {
        return json!({ "ok": false, "error": "ask_user_question requires non-empty 'questions'" })
            .to_string();
    }

    json!({
        "type": "needs_user_input",
        "context": context,
        "questions": questions,
        "allow_freeform": allow_freeform,
        "instruction": "Stop tool execution and ask the user this question before continuing."
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
