use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, false)
}

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
                "question": {
                    "type": "string",
                    "description": "Legacy single-question field"
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
            "anyOf": [
                { "required": ["questions"] },
                { "required": ["question"] }
            ]
        }),
    }
}

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
        .or_else(|| question.get("multiSelect"))
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

pub fn execute(input: Value) -> String {
    let context = input
        .get("context")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let allow_freeform = input
        .get("allow_freeform")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut questions = input
        .get("questions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(normalize_question).collect::<Vec<_>>())
        .unwrap_or_default();

    if questions.is_empty() {
        let question = match input.get("question").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q.trim().to_string(),
            _ => return "Error: Missing or empty 'question' argument".into(),
        };

        let options: Vec<Value> = input
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(|label| {
                        json!({
                            "label": label,
                            "description": "Select this option"
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if options.len() < 2 {
            return "Error: ask_user_question requires at least two options".into();
        }

        questions.push(json!({
            "question": question,
            "header": "Clarify",
            "multi_select": false,
            "options": options
        }));
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
