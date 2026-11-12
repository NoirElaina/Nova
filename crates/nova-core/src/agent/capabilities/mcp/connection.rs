use super::elicitation;
use super::{McpNegotiatedCapabilities, McpResourceInfo, McpToolInfo};
use crate::core::Core;
use async_trait::async_trait;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig,
    ElicitRequestParams, ElicitResult, ElicitationCapability, ErrorData as McpError,
    Implementation, JsonObject, ProtocolVersion, ReadResourceRequestParams, ServerResult,
};
use rmcp::service::{
    NotificationContext, Peer, PeerRequestOptions, RequestContext, RoleClient, RunningService,
    ServiceError,
};
use rmcp::ClientHandler;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub(super) enum McpCallError {
    Timeout(String),
    Cancelled(String),
    Protocol(String),
    Transport(String),
}

impl McpCallError {
    pub(super) fn into_message(self) -> String {
        match self {
            Self::Timeout(msg)
            | Self::Cancelled(msg)
            | Self::Protocol(msg)
            | Self::Transport(msg) => msg,
        }
    }

    pub(super) fn should_reconnect(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

#[async_trait]
pub(super) trait McpConnection: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String>;
    async fn list_resources(&self) -> Result<Vec<McpResourceInfo>, String>;
    async fn read_resource(&self, uri: &str) -> Result<Value, String>;
    async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        cancel_token: Option<CancellationToken>,
        call_timeout: Duration,
    ) -> Result<Value, McpCallError>;
    fn negotiated_info(&self) -> (Option<String>, Option<McpNegotiatedCapabilities>);
    async fn shutdown(&self);
}

#[derive(Clone)]
pub(super) struct NovaMcpHandler {
    core: Core,
    server_name: String,
}

impl NovaMcpHandler {
    pub(super) fn new(core: Core, server_name: impl Into<String>) -> Self {
        Self {
            core,
            server_name: server_name.into(),
        }
    }
}

impl ClientHandler for NovaMcpHandler {
    fn get_info(&self) -> ClientConfig {
        let mut capabilities = ClientCapabilities::default();
        capabilities.elicitation = Some(
            ElicitationCapability::new()
                .with_form(Default::default())
                .with_url(Default::default()),
        );
        ClientConfig::new(
            capabilities,
            Implementation::new("Nova", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, McpError> {
        elicitation::request_elicitation(&self.core, &self.server_name, request, context.ct).await
    }

    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        let core = self.core.clone();
        let server_name = self.server_name.clone();
        let peer = context.peer;
        tokio::spawn(async move {
            super::handle_tool_list_changed_notification(core, server_name, peer).await;
        });
    }
}

pub(super) struct RmcpConnection {
    peer: Peer<RoleClient>,
    service: Mutex<Option<RunningService<RoleClient, NovaMcpHandler>>>,
    protocol_version: Option<String>,
    capabilities: Option<McpNegotiatedCapabilities>,
}

impl RmcpConnection {
    pub(super) fn from_running_service(
        running: RunningService<RoleClient, NovaMcpHandler>,
    ) -> Arc<dyn McpConnection> {
        let peer = running.peer().clone();
        let (protocol_version, capabilities) = extract_negotiated_info(&peer);
        Arc::new(Self {
            peer,
            service: Mutex::new(Some(running)),
            protocol_version,
            capabilities,
        })
    }
}

fn extract_negotiated_info(
    peer: &Peer<RoleClient>,
) -> (Option<String>, Option<McpNegotiatedCapabilities>) {
    let Some(info) = peer.peer_info() else {
        return (None, None);
    };
    let version = Some(info.protocol_version.as_str().to_string());
    let caps = McpNegotiatedCapabilities {
        tools: info.capabilities.tools.is_some(),
        resources: info.capabilities.resources.is_some(),
        prompts: info.capabilities.prompts.is_some(),
        logging: info.capabilities.logging.is_some(),
        tools_list_changed: info
            .capabilities
            .tools
            .as_ref()
            .and_then(|t| t.list_changed)
            .unwrap_or(false),
    };
    (version, Some(caps))
}

pub(super) fn map_rmcp_tools(tools: Vec<rmcp::model::Tool>) -> Vec<McpToolInfo> {
    tools
        .into_iter()
        .map(|tool| McpToolInfo {
            name: tool.name.into_owned(),
            description: tool.description.map(|d| d.into_owned()),
            input_schema: Some(Value::Object(tool.input_schema.as_ref().clone())),
        })
        .collect()
}

pub(super) fn map_call_tool_result(result: CallToolResult) -> Result<Value, String> {
    serde_json::to_value(result).map_err(|e| format!("Failed to serialize MCP tool result: {e}"))
}

fn normalize_tool_arguments(arguments: Value) -> Result<Option<JsonObject>, String> {
    match arguments {
        Value::Null => Ok(None),
        Value::Object(map) => Ok(Some(map)),
        other => Err(format!(
            "MCP tools/call arguments must be a JSON object or null, got: {other}"
        )),
    }
}

fn format_service_error(err: ServiceError) -> String {
    match err {
        ServiceError::McpError(e) => format!("MCP error ({}): {}", e.code.0, e.message),
        other => other.to_string(),
    }
}

#[async_trait]
impl McpConnection for RmcpConnection {
    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        let tools = self
            .peer
            .list_all_tools()
            .await
            .map_err(format_service_error)?;
        Ok(map_rmcp_tools(tools))
    }

    async fn list_resources(&self) -> Result<Vec<McpResourceInfo>, String> {
        let resources = self
            .peer
            .list_all_resources()
            .await
            .map_err(format_service_error)?;
        Ok(resources
            .into_iter()
            .map(|r| McpResourceInfo {
                uri: r.uri,
                name: r.name,
                description: r.description,
                mime_type: r.mime_type,
            })
            .collect())
    }

    async fn read_resource(&self, uri: &str) -> Result<Value, String> {
        let res = self
            .peer
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(format_service_error)?;
        serde_json::to_value(res)
            .map_err(|e| format!("Failed to serialize MCP resource result: {e}"))
    }

    async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        cancel_token: Option<CancellationToken>,
        call_timeout: Duration,
    ) -> Result<Value, McpCallError> {
        let mut params = CallToolRequestParams::new(tool_name.to_string());
        if let Some(args) = normalize_tool_arguments(arguments).map_err(McpCallError::Protocol)? {
            params = params.with_arguments(args);
        }

        let mut handle = self
            .peer
            .send_cancellable_request(
                rmcp::model::ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .map_err(|e| match e {
                ServiceError::McpError(_) => McpCallError::Protocol(format_service_error(e)),
                other => McpCallError::Transport(format_service_error(other)),
            })?;

        let response = tokio::select! {
            biased;
            _ = async {
                match cancel_token.as_ref() {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => {
                let _ = handle.cancel(Some("Request cancelled by user".to_string())).await;
                return Err(McpCallError::Cancelled(
                    format!("MCP tool call '{}' cancelled", tool_name),
                ));
            }
            _ = tokio::time::sleep(call_timeout) => {
                let _ = handle.cancel(Some("Request timed out".to_string())).await;
                return Err(McpCallError::Timeout(
                    format!("MCP tools/call timeout ({}s)", call_timeout.as_secs()),
                ));
            }
            res = &mut handle.rx => {
                match res {
                    Ok(Ok(server_res)) => server_res,
                    Ok(Err(service_err)) => {
                        return Err(match service_err {
                            ServiceError::McpError(_) => {
                                McpCallError::Protocol(format_service_error(service_err))
                            }
                            other => McpCallError::Transport(format_service_error(other)),
                        });
                    }
                    Err(_) => {
                        return Err(McpCallError::Transport(
                            "MCP request channel closed unexpectedly".to_string(),
                        ));
                    }
                }
            }
        };

        match response {
            ServerResult::CallToolResult(res) => map_call_tool_result(res)
                .map_err(|e| McpCallError::Protocol(format!("Invalid MCP tool result: {e}"))),
            other => serde_json::to_value(other)
                .map_err(|e| McpCallError::Protocol(format!("Unexpected MCP response: {e}"))),
        }
    }

    fn negotiated_info(&self) -> (Option<String>, Option<McpNegotiatedCapabilities>) {
        (self.protocol_version.clone(), self.capabilities.clone())
    }

    async fn shutdown(&self) {
        let service = self.service.lock().await.take();
        if let Some(svc) = service {
            let _ = svc.cancel().await;
        }
        let _ = json!({});
    }
}
