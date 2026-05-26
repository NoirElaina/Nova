use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequest {
    pub(crate) model: String,
    pub(crate) input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<ResponsesTool>>,
    pub(crate) max_output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<ResponsesToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parallel_tool_calls: Option<bool>,
    pub(crate) truncation: ResponsesTruncation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<ResponsesReasoningRequest>,
    pub(crate) text: ResponsesTextConfig,
    pub(crate) stream: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesInputItem {
    #[serde(rename = "message")]
    Message {
        role: String,
        content: Vec<ResponsesContentPart>,
    },

    #[serde(rename = "function_call")]
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },

    #[serde(rename = "function_call_output")]
    FunctionCallOutput { call_id: String, output: String },

    #[serde(rename = "reasoning")]
    Reasoning {
        summary: Vec<ResponsesReasoningSummaryPart>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesContentPart {
    #[serde(rename = "input_text")]
    InputText { text: String },

    #[serde(rename = "output_text")]
    OutputText { text: String },

    #[serde(rename = "input_image")]
    InputImage { image_url: String },
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesTool {
    pub(crate) r#type: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponsesToolChoice {
    Auto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponsesTruncation {
    Disabled,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesReasoningRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesTextConfig {
    pub(crate) format: ResponsesTextFormat,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesTextFormat {
    #[serde(rename = "type")]
    pub(crate) format_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub(crate) enum ResponsesReasoningSummaryPart {
    #[serde(rename = "summary_text")]
    SummaryText { text: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesStreamEvent {
    #[serde(rename = "response.created")]
    Created {
        #[serde(rename = "response")]
        _response: ResponsesResponse,
    },

    #[serde(rename = "response.queued")]
    Queued {
        #[serde(rename = "response")]
        _response: ResponsesResponse,
    },

    #[serde(rename = "response.in_progress")]
    InProgress {
        #[serde(rename = "response")]
        _response: ResponsesResponse,
    },

    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: usize,
        item: ResponsesOutputItem,
    },

    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        #[serde(rename = "output_index")]
        _output_index: usize,
        #[serde(rename = "content_index")]
        _content_index: usize,
    },

    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        #[serde(rename = "output_index")]
        _output_index: usize,
        #[serde(rename = "content_index")]
        _content_index: usize,
    },

    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta { delta: String },

    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        #[serde(rename = "output_index")]
        _output_index: usize,
        #[serde(rename = "content_index")]
        _content_index: usize,
        #[serde(rename = "text")]
        _text: String,
    },

    #[serde(rename = "response.refusal.delta")]
    RefusalDelta { delta: String },

    #[serde(rename = "response.refusal.done")]
    RefusalDone {
        #[serde(rename = "output_index")]
        _output_index: usize,
        #[serde(rename = "content_index")]
        _content_index: usize,
        #[serde(rename = "refusal")]
        _refusal: String,
    },

    #[serde(rename = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded {
        output_index: usize,
        #[serde(rename = "summary_index")]
        _summary_index: usize,
    },

    #[serde(rename = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone {
        output_index: usize,
        #[serde(rename = "summary_index")]
        _summary_index: usize,
        part: Option<ResponsesReasoningSummaryPart>,
    },

    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        #[serde(default)]
        output_index: usize,
        #[serde(default)]
        #[serde(rename = "summary_index")]
        _summary_index: usize,
        delta: String,
    },

    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryTextDone {
        #[serde(default)]
        output_index: usize,
        #[serde(default)]
        #[serde(rename = "summary_index")]
        _summary_index: usize,
        text: String,
    },

    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta { output_index: usize, delta: String },

    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        output_index: usize,
        arguments: String,
    },

    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        output_index: usize,
        item: ResponsesOutputItem,
    },

    #[serde(rename = "response.completed")]
    Completed { response: ResponsesResponse },

    #[serde(rename = "response.failed")]
    Failed { response: ResponsesResponse },

    #[serde(rename = "response.incomplete")]
    Incomplete { response: ResponsesResponse },

    #[serde(rename = "response.output_item.failed")]
    OutputItemFailed {
        output_index: usize,
        item: Option<ResponsesOutputItem>,
    },

    #[serde(rename = "error")]
    Error {
        code: Option<String>,
        message: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesOutputItem {
    #[serde(rename = "type")]
    pub(crate) item_type: String,
    #[serde(rename = "id")]
    pub(crate) _id: Option<String>,
    #[serde(rename = "status")]
    pub(crate) _status: Option<String>,
    pub(crate) call_id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
    pub(crate) summary: Option<Vec<ResponsesReasoningSummaryPart>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesResponse {
    pub(crate) id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) usage: Option<ResponsesUsage>,
    pub(crate) error: Option<ResponsesResponseError>,
    pub(crate) incomplete_details: Option<ResponsesIncompleteDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesUsage {
    pub(crate) input_tokens: Option<u32>,
    pub(crate) output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesResponseError {
    pub(crate) code: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesIncompleteDetails {
    pub(crate) reason: Option<String>,
}
