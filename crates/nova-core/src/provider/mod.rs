pub mod adapters;
pub mod client;
pub mod model_fetch;
pub mod pricing;
pub mod sse_utils;
pub mod stream_runner;
pub mod token_counter;
pub mod token_usage_log;
pub mod types;

pub use client::LlmClient;
pub use types::*;

use crate::agent::tools::ToolCallRequest;
use crate::provider::pricing::TurnCostBreakdown;

#[derive(Debug, Clone)]
pub struct ProviderTurnResult {
    pub messages: Vec<Message>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_creation_tokens: Option<u32>,
    pub cost: Option<TurnCostBreakdown>,
    pub prevent_continuation: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderPromptEstimate {
    pub input_tokens: u32,
    pub source: &'static str,
    pub tool_count: usize,
}

pub use nova_protocol::errors::NovaErrorInfo;

#[derive(Debug, Clone)]
pub struct ProviderTurnError {
    pub message: String,
    pub error_info: NovaErrorInfo,
    pub partial_messages: Vec<Message>,
}

impl ProviderTurnError {
    pub fn new(message: String) -> Self {
        let error_info = classify_error(&message, None);
        Self {
            message,
            error_info,
            partial_messages: Vec::new(),
        }
    }

    pub fn with_info(message: String, error_info: NovaErrorInfo) -> Self {
        Self {
            message,
            error_info,
            partial_messages: Vec::new(),
        }
    }

    pub fn with_partial(message: String, partial_messages: Vec<Message>) -> Self {
        let error_info = classify_error(&message, None);
        Self {
            message,
            error_info,
            partial_messages,
        }
    }

    pub fn with_partial_and_info(
        message: String,
        error_info: NovaErrorInfo,
        partial_messages: Vec<Message>,
    ) -> Self {
        Self {
            message,
            error_info,
            partial_messages,
        }
    }

    pub fn from_status_and_body(status: reqwest::StatusCode, body: &str) -> Self {
        let code = status.as_u16();
        let retry_after_ms = parse_retry_after(body);
        let error_info = if code == 429 {
            NovaErrorInfo::RateLimitExceeded { retry_after_ms }
        } else if code == 503 || code == 529 {
            NovaErrorInfo::ServerOverloaded { retry_after_ms }
        } else if code == 401 || code == 403 {
            NovaErrorInfo::Unauthorized
        } else if code == 404 {
            NovaErrorInfo::ModelNotFound
        } else if code == 400
            && (body.contains("maximum context length")
                || body.contains("context_length_exceeded")
                || body.contains("prompt_too_long"))
        {
            NovaErrorInfo::ContextWindowExceeded
        } else if code >= 500 {
            NovaErrorInfo::InternalServerError
        } else if code >= 400 {
            NovaErrorInfo::BadRequest
        } else {
            NovaErrorInfo::HttpConnectionFailed {
                http_status_code: Some(code),
            }
        };

        Self {
            message: format!("API Error [{}] => {}", code, body),
            error_info,
            partial_messages: Vec::new(),
        }
    }
}

impl std::fmt::Display for ProviderTurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProviderTurnError {}

fn parse_retry_after(body: &str) -> Option<u64> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(sec) = v.get("retry_after").and_then(|s| s.as_f64()) {
            return Some((sec * 1000.0) as u64);
        }
        if let Some(sec) = v.get("error").and_then(|e| e.get("retry_after")).and_then(|s| s.as_f64()) {
            return Some((sec * 1000.0) as u64);
        }
    }
    None
}

/// Canonical check for mid-stream transport/decode disconnect errors shared between
/// `classify_error` (retry policy) and `stream_runner` (graceful EOF after `stop_reason`).
pub(crate) fn is_disconnect_error_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("stream socket closed")
        || lower.contains("error decoding response body")
        || lower.contains("unexpected eof")
        || lower.contains("connection closed")
        || lower.contains("connection reset")
        || lower.contains("forcibly closed")
        || lower.contains("wsarecv")
        || lower.contains("stream reading error")
        || lower.contains("broken pipe")
        || lower.contains("reset by peer")
        || lower.contains("connection abort")
        || lower.contains("10054")
}

fn classify_error(msg: &str, status: Option<u16>) -> NovaErrorInfo {
    let lower = msg.to_lowercase();
    if lower.contains("rate limit") || lower.contains("429") || lower.contains("too many requests") {
        NovaErrorInfo::RateLimitExceeded { retry_after_ms: None }
    } else if lower.contains("overloaded") || lower.contains("503") || lower.contains("529") {
        NovaErrorInfo::ServerOverloaded { retry_after_ms: None }
    } else if lower.contains("context length") || lower.contains("maximum context") || lower.contains("prompt_too_long") {
        NovaErrorInfo::ContextWindowExceeded
    } else if lower.contains("unauthorized") || lower.contains("401") || lower.contains("invalid api key") {
        NovaErrorInfo::Unauthorized
    } else if lower.contains("cancelled") {
        NovaErrorInfo::Cancelled
    } else if is_disconnect_error_message(&lower) {
        NovaErrorInfo::ResponseStreamDisconnected { http_status_code: status }
    } else if lower.contains("connection refused") || lower.contains("connect error") || lower.contains("failed to connect") {
        NovaErrorInfo::ResponseStreamConnectionFailed { http_status_code: status }
    } else {
        NovaErrorInfo::Other(msg.to_string())
    }
}

impl From<String> for ProviderTurnError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification_and_retryability() {
        let err_429 = ProviderTurnError::from_status_and_body(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            r#"{"error": {"message": "Rate limit", "retry_after": 2.5}}"#,
        );
        assert!(matches!(err_429.error_info, NovaErrorInfo::RateLimitExceeded { retry_after_ms: Some(2500) }));
        assert!(err_429.error_info.is_retryable());
        assert_eq!(err_429.error_info.default_retry_delay(1).as_millis(), 2500);

        let err_503 = ProviderTurnError::from_status_and_body(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            "Server is overloaded",
        );
        assert!(matches!(err_503.error_info, NovaErrorInfo::ServerOverloaded { .. }));
        assert!(err_503.error_info.is_retryable());
        assert_eq!(err_503.error_info.default_retry_delay(1).as_millis(), 500);
        assert_eq!(err_503.error_info.default_retry_delay(2).as_millis(), 1000);
        assert_eq!(err_503.error_info.default_retry_delay(3).as_millis(), 2000);

        let err_401 = ProviderTurnError::from_status_and_body(
            reqwest::StatusCode::UNAUTHORIZED,
            "Invalid API key",
        );
        assert_eq!(err_401.error_info, NovaErrorInfo::Unauthorized);
        assert!(!err_401.error_info.is_retryable());

        let err_400_context = ProviderTurnError::from_status_and_body(
            reqwest::StatusCode::BAD_REQUEST,
            "maximum context length exceeded",
        );
        assert_eq!(err_400_context.error_info, NovaErrorInfo::ContextWindowExceeded);
        assert!(!err_400_context.error_info.is_retryable());

        for sample in [
            "error decoding response body: unexpected EOF",
            "An existing connection was forcibly closed by the remote host (os error 10054)",
            "wsarecv: connection abort",
            "stream reading error: broken pipe",
        ] {
            let err = ProviderTurnError::new(sample.to_string());
            assert!(
                matches!(err.error_info, NovaErrorInfo::ResponseStreamDisconnected { .. }),
                "expected ResponseStreamDisconnected for {sample:?}, got {:?}",
                err.error_info
            );
            assert!(err.error_info.is_retryable());
        }
    }
}
