use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequest {
    pub(crate) model: String,
    pub(crate) input: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<ResponsesTool>>,
    pub(crate) stream: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesTool {
    pub(crate) r#type: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesStreamEvent {
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: usize,
        item: ResponsesOutputItem,
    },

    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta { delta: String },

    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta { delta: String },

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

    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesOutputItem {
    #[serde(rename = "type")]
    pub(crate) item_type: String,
    pub(crate) call_id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesResponse {
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
