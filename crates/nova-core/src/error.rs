//! 内核统一结构化错误体系。
//!
//! 领域层统一产出的 `NovaCoreError` 在边界处无损映射为：
//! - 事件流给前端的结构化 `NovaErrorInfo`；
//! - JSON-RPC `(code, message)`；
//! - 面向 UI 的人类可读原文 `Display`。

use nova_protocol::errors::NovaErrorInfo;

use crate::provider::ProviderTurnError;

/// JSON-RPC 应用层语义错误码区间（-32000..-32099，遵循 JSON-RPC 2.0 规范）。
pub const RPC_NOT_FOUND: i32 = -32002;
pub const RPC_PERMISSION_DENIED: i32 = -32003;
pub const RPC_CANCELLED: i32 = -32004;
pub const RPC_PROVIDER_ERROR: i32 = -32005;
pub const RPC_TOOL_ERROR: i32 = -32006;
pub const RPC_INVALID_PARAMS: i32 = -32602;
pub const RPC_INTERNAL_ERROR: i32 = -32603;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NovaCoreError {
    #[error("{message}")]
    Provider {
        message: String,
        error_info: NovaErrorInfo,
    },

    #[error("{0}")]
    InvalidInput(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    PermissionDenied(String),

    #[error("Tool '{tool}' failed: {message}")]
    ToolFailed { tool: String, message: String },

    #[error("Sandbox '{driver}' error: {message}")]
    Sandbox {
        driver: &'static str,
        message: String,
    },

    #[error("MCP '{server}' error: {message}")]
    Mcp { server: String, message: String },

    #[error("Session storage error: {0}")]
    Session(String),

    #[error("Cancelled")]
    Cancelled,

    #[error("{0}")]
    Internal(String),
}

impl NovaCoreError {
    pub fn provider(message: impl Into<String>, error_info: NovaErrorInfo) -> Self {
        Self::Provider {
            message: message.into(),
            error_info,
        }
    }

    pub fn sandbox(driver: &'static str, message: impl Into<String>) -> Self {
        Self::Sandbox {
            driver,
            message: message.into(),
        }
    }

    pub fn mcp(server: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Mcp {
            server: server.into(),
            message: message.into(),
        }
    }

    /// 映射为协议层暴露给前端的结构化 `NovaErrorInfo`。
    pub fn to_nova_error_info(&self) -> NovaErrorInfo {
        match self {
            Self::Provider { error_info, .. } => error_info.clone(),
            Self::InvalidInput(_) => NovaErrorInfo::BadRequest,
            Self::PermissionDenied(reason) => NovaErrorInfo::PermissionDenied {
                reason: reason.clone(),
            },
            Self::ToolFailed { tool, .. } => NovaErrorInfo::ToolExecutionFailed {
                tool_name: tool.clone(),
            },
            Self::Cancelled => NovaErrorInfo::Cancelled,
            Self::NotFound(_)
            | Self::Sandbox { .. }
            | Self::Mcp { .. }
            | Self::Session(_)
            | Self::Internal(_) => NovaErrorInfo::Other(self.to_string()),
        }
    }

    /// 映射为 JSON-RPC 错误码（用于 CLI daemon / 桌面 IPC 边界）。
    pub fn json_rpc_code(&self) -> i32 {
        match self {
            Self::InvalidInput(_) => RPC_INVALID_PARAMS,
            Self::NotFound(_) => RPC_NOT_FOUND,
            Self::PermissionDenied(_) => RPC_PERMISSION_DENIED,
            Self::Cancelled => RPC_CANCELLED,
            Self::Provider { .. } => RPC_PROVIDER_ERROR,
            Self::ToolFailed { .. } | Self::Sandbox { .. } | Self::Mcp { .. } => RPC_TOOL_ERROR,
            Self::Session(_) | Self::Internal(_) => RPC_INTERNAL_ERROR,
        }
    }

    pub fn to_json_rpc_error(&self) -> (i32, String) {
        (self.json_rpc_code(), self.to_string())
    }
}

impl From<ProviderTurnError> for NovaCoreError {
    fn from(err: ProviderTurnError) -> Self {
        Self::Provider {
            message: err.message,
            error_info: err.error_info,
        }
    }
}

impl From<sqlx::Error> for NovaCoreError {
    fn from(err: sqlx::Error) -> Self {
        Self::Session(err.to_string())
    }
}

impl From<serde_json::Error> for NovaCoreError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidInput(err.to_string())
    }
}

impl From<NovaCoreError> for String {
    fn from(err: NovaCoreError) -> Self {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_variants_to_protocol_info_and_rpc_codes() {
        let provider_err = NovaCoreError::from(ProviderTurnError::with_info(
            "429".to_string(),
            NovaErrorInfo::RateLimitExceeded {
                retry_after_ms: Some(1500),
            },
        ));
        assert_eq!(
            provider_err.to_nova_error_info(),
            NovaErrorInfo::RateLimitExceeded {
                retry_after_ms: Some(1500)
            }
        );
        assert_eq!(provider_err.json_rpc_code(), RPC_PROVIDER_ERROR);

        let perm = NovaCoreError::PermissionDenied("protected path".to_string());
        assert_eq!(
            perm.to_nova_error_info(),
            NovaErrorInfo::PermissionDenied {
                reason: "protected path".to_string()
            }
        );
        assert_eq!(perm.json_rpc_code(), RPC_PERMISSION_DENIED);

        let tool = NovaCoreError::ToolFailed {
            tool: "Bash".to_string(),
            message: "exit 1".to_string(),
        };
        assert_eq!(
            tool.to_nova_error_info(),
            NovaErrorInfo::ToolExecutionFailed {
                tool_name: "Bash".to_string()
            }
        );
        assert_eq!(tool.json_rpc_code(), RPC_TOOL_ERROR);

        assert_eq!(
            NovaCoreError::NotFound("conv-1".into()).json_rpc_code(),
            RPC_NOT_FOUND
        );
        assert_eq!(
            NovaCoreError::InvalidInput("bad arg".into()).json_rpc_code(),
            RPC_INVALID_PARAMS
        );
        assert_eq!(
            NovaCoreError::Cancelled.json_rpc_code(),
            RPC_CANCELLED
        );
        assert_eq!(
            NovaCoreError::Session("sqlite busy".into()).json_rpc_code(),
            RPC_INTERNAL_ERROR
        );
    }
}
