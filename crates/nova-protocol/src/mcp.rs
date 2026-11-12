use ts_rs::TS;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// MCP 连接在运行时对外暴露的状态。
pub enum McpRuntimeStatus {
    Disconnected,
    Connected,
    Error,
}

impl McpRuntimeStatus {
    // 转成前端和设置层使用的稳定字符串值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connected => "connected",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, TS)]
#[serde(tag = "type", rename_all = "lowercase")]
// 持久化的 MCP 服务配置。
pub enum McpServerConfig {
    // 本地子进程，通过 stdio 与 MCP 服务通信。
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    // 预留的 SSE 连接配置。
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    #[serde(
        rename = "streamable_http",
        alias = "streamablehttp",
        alias = "streamable-http"
    )]
    // 基于 streamable HTTP 的远程服务配置。
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, TS)]
#[serde(rename_all = "camelCase")]
// 返回给前端的 MCP 服务配置项。
pub struct McpServerEntry {
    pub name: String,
    pub enabled: bool,
    pub config: McpServerConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
/// 协商后记录的服务端能力摘要。
pub struct McpNegotiatedCapabilities {
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub resources: bool,
    #[serde(default)]
    pub prompts: bool,
    #[serde(default)]
    pub logging: bool,
    #[serde(default)]
    pub tools_list_changed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
/// OAuth 登录/登出结果。
pub struct McpOAuthLoginResult {
    pub status: String,
    pub auth_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
/// 前端回传给 MCP 服务端的 Elicitation 用户交互裁决。
pub struct McpElicitationDecision {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
// 返回给前端的服务运行状态快照。
pub struct McpServerStatus {
    pub name: String,
    pub status: String,
    pub enabled: bool,
    pub r#type: String,
    #[ts(type = "number")]
    pub tool_count: usize,
    pub error: Option<String>,
    /// 来源智能体名（由智能体配置页创建/迁入）；普通全局条目为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    /// 协商后的 MCP 协议版本（如 `2025-11-25` / `2024-11-05`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    /// 协商后的服务端能力摘要。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<McpNegotiatedCapabilities>,
    /// 远端服务 OAuth 状态（`none` / `unauthenticated` / `authenticated` / `error`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, TS)]
#[serde(rename_all = "camelCase")]
// MCP 工具的基础元数据。
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, TS)]
#[serde(rename_all = "camelCase")]
// MCP 资源列表项的基础信息。
pub struct McpResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}
