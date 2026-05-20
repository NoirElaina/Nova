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

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    },
    #[serde(
        rename = "streamable_http",
        alias = "streamablehttp",
        alias = "streamable-http"
    )]
    // 基于 streamable HTTP 的远程服务配置。
    StreamableHttp {
        url: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
// 返回给前端的 MCP 服务配置项。
pub struct McpServerEntry {
    pub name: String,
    pub enabled: bool,
    pub config: McpServerConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
// 返回给前端的服务运行状态快照。
pub struct McpServerStatus {
    pub name: String,
    pub status: String,
    pub enabled: bool,
    pub r#type: String,
    pub tool_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
// MCP 工具的基础元数据。
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
// MCP 资源列表项的基础信息。
pub struct McpResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}
