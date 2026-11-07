use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 标准 JSON-RPC 2.0 基础请求信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// 标准 JSON-RPC 2.0 基础响应信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 错误格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// 服务端主动推送的事件通知信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

/// 当前线协议版本。客户端与服务端在 `initialize` 阶段交换，
/// 不匹配时由宿主显式告警，而不是靠「覆盖二进制」隐式兜底。
pub const PROTOCOL_VERSION: &str = "1";

/// `initialize` 请求参数（与 Codex App Server 的握手同形）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    /// 客户端声明的能力，例如 `["browser"]`。
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// 一端（客户端或服务端）的身份描述。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
pub struct ClientInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub version: String,
}

/// `initialize` 响应：服务端自报身份、协议版本与可用能力。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
pub struct InitializeResult {
    pub server_info: ClientInfo,
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}
