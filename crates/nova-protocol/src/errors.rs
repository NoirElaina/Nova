//! 强类型错误模型与重试状态机类型定义（对齐工业级 Agent 协议规范）。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
pub enum NovaErrorInfo {
    ContextWindowExceeded,
    UsageLimitExceeded,
    RateLimitExceeded {
        #[ts(type = "number | null")]
        retry_after_ms: Option<u64>,
    },
    ServerOverloaded {
        #[ts(type = "number | null")]
        retry_after_ms: Option<u64>,
    },
    ModelNotFound,
    Unauthorized,
    BadRequest,
    InternalServerError,
    HttpConnectionFailed { http_status_code: Option<u16> },
    ResponseStreamConnectionFailed { http_status_code: Option<u16> },
    ResponseStreamDisconnected { http_status_code: Option<u16> },
    ResponseTooManyFailedAttempts { http_status_code: Option<u16> },
    ToolExecutionFailed { tool_name: String },
    PermissionDenied { reason: String },
    Cancelled,
    Other(String),
}

impl NovaErrorInfo {
    /// 判断该错误是否属于可自动重试的瞬态错误。
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            NovaErrorInfo::RateLimitExceeded { .. }
                | NovaErrorInfo::ServerOverloaded { .. }
                | NovaErrorInfo::HttpConnectionFailed { .. }
                | NovaErrorInfo::ResponseStreamConnectionFailed { .. }
                | NovaErrorInfo::ResponseStreamDisconnected { .. }
        )
    }

    /// 根据重试轮次计算默认指数退避等待延迟。
    pub fn default_retry_delay(&self, attempt: u32) -> std::time::Duration {
        match self {
            NovaErrorInfo::RateLimitExceeded {
                retry_after_ms: Some(ms),
            }
            | NovaErrorInfo::ServerOverloaded {
                retry_after_ms: Some(ms),
            } => std::time::Duration::from_millis((*ms).max(500)),
            _ => {
                // 指数退避：500ms -> 1s -> 2s -> 4s -> max 10s
                let base_ms = 500u64;
                let multiplier = 2u64.saturating_pow(attempt.saturating_sub(1));
                let delay_ms = (base_ms.saturating_mul(multiplier)).min(10_000);
                std::time::Duration::from_millis(delay_ms)
            }
        }
    }
}
