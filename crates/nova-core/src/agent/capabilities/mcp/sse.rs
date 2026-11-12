use super::connection::{McpConnection, NovaMcpHandler, RmcpConnection};
use super::streamable_http::{is_unauthorized_error_text, split_custom_headers};
use super::{oauth, MCP_CONNECT_TIMEOUT};
use crate::core::Core;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::{HeaderName, HeaderValue};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Url;
use rmcp::model::ServerJsonRpcMessage;
use rmcp::service::ServiceExt;
use rmcp::transport::worker::{Worker, WorkerConfig, WorkerContext, WorkerQuitReason, WorkerSendRequest};
use rmcp::RoleClient;
use sse_stream::{Error as SseParseError, Sse, SseStream};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::time::timeout;

#[derive(Debug, Error)]
pub(super) enum LegacySseError {
    #[error("Legacy SSE transport closed")]
    Closed,
    #[error("Legacy SSE task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("Legacy SSE error: {0}")]
    Message(String),
}

struct LegacySseHttp {
    core: Core,
    server_name: String,
    server_url: String,
    client: reqwest::Client,
    custom_headers: HashMap<HeaderName, HeaderValue>,
    manual_bearer: Option<String>,
    post_url: String,
}

struct LegacySseWorker {
    http: LegacySseHttp,
    sse_stream: BoxStream<'static, Result<Sse, SseParseError>>,
}

impl LegacySseHttp {
    fn resolve_endpoint(base_url: &str, raw_endpoint: &str) -> Result<String, String> {
        let trimmed = raw_endpoint.trim();
        if trimmed.is_empty() {
            return Err("SSE endpoint event returned empty URL".to_string());
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Ok(trimmed.to_string());
        }
        let base = Url::parse(base_url).map_err(|e| format!("Invalid SSE base URL: {e}"))?;
        let joined = base
            .join(trimmed)
            .map_err(|e| format!("Invalid SSE endpoint path '{}': {e}", trimmed))?;
        Ok(joined.to_string())
    }

    async fn resolve_auth_token(&self) -> Result<Option<String>, LegacySseError> {
        match oauth::resolve_bearer_token_if_configured(
            &self.core,
            &self.server_name,
            &self.server_url,
        )
        .await
        {
            Ok(Some(token)) => Ok(Some(token)),
            Ok(None) => Ok(self.manual_bearer.clone()),
            Err(e) => Err(LegacySseError::Message(e)),
        }
    }

    async fn send_post_request(
        &self,
        req: WorkerSendRequest<LegacySseWorker>,
        context: &WorkerContext<LegacySseWorker>,
    ) -> Result<(), WorkerQuitReason<LegacySseError>> {
        let body = serde_json::to_vec(&req.message).map_err(|e| {
            WorkerQuitReason::fatal(
                LegacySseError::Message(format!("Failed to serialize MCP message: {e}")),
                "serialize legacy SSE message",
            )
        })?;

        let bearer = match self.resolve_auth_token().await {
            Ok(b) => b,
            Err(e) => {
                let _ = req.responder.send(Err(e));
                return Ok(());
            }
        };

        let mut builder = self
            .client
            .post(&self.post_url)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .body(body);
        for (name, val) in &self.custom_headers {
            builder = builder.header(name, val);
        }
        if let Some(token) = bearer {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }

        let resp = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = req.responder.send(Err(LegacySseError::Message(e.to_string())));
                return Ok(());
            }
        };

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            oauth::mark_server_auth_required(&self.core, &self.server_name, &self.server_url);
            let _ = req.responder.send(Err(LegacySseError::Message(
                "MCP SSE POST unauthorized (401); OAuth sign-in required".to_string(),
            )));
            return Ok(());
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let _ = req.responder.send(Err(LegacySseError::Message(format!(
                "MCP SSE POST failed ({}): {}",
                status.as_u16(),
                text
            ))));
            return Ok(());
        }

        let _ = req.responder.send(Ok(()));

        // 兼容部分 SSE 服务端在 POST 响应体内直接回包 JSON-RPC 响应。
        let resp_text = resp.text().await.unwrap_or_default();
        let trimmed = resp_text.trim();
        if !trimmed.is_empty() {
            if let Ok(msg) = serde_json::from_str::<ServerJsonRpcMessage>(trimmed) {
                let _ = context.to_handler_tx.send(msg).await;
            }
        }
        Ok(())
    }
}

impl Worker for LegacySseWorker {
    type Error = LegacySseError;
    type Role = RoleClient;

    fn err_closed() -> Self::Error {
        LegacySseError::Closed
    }

    fn err_join(e: tokio::task::JoinError) -> Self::Error {
        LegacySseError::Join(e)
    }

    fn config(&self) -> WorkerConfig {
        let mut cfg = WorkerConfig::default();
        cfg.name = Some(format!("nova-mcp-sse:{}", self.http.server_name));
        cfg.channel_buffer_capacity = 64;
        cfg
    }

    fn is_control_message(message: &rmcp::service::TxJsonRpcMessage<Self::Role>) -> bool {
        matches!(
            message,
            rmcp::model::JsonRpcMessage::Notification(notif)
                if matches!(
                    notif.notification,
                    rmcp::model::ClientNotification::CancelledNotification(_)
                )
        ) || matches!(message, rmcp::model::JsonRpcMessage::Response(_))
    }

    async fn run(
        self,
        mut context: WorkerContext<Self>,
    ) -> Result<(), WorkerQuitReason<Self::Error>> {
        let LegacySseWorker {
            mut http,
            mut sse_stream,
        } = self;
        loop {
            tokio::select! {
                biased;
                _ = context.cancellation_token.cancelled() => {
                    return Err(WorkerQuitReason::Cancelled);
                }
                maybe_ctrl = context.control_from_handler_rx.recv() => {
                    let Some(req) = maybe_ctrl else {
                        return Err(WorkerQuitReason::HandlerTerminated);
                    };
                    http.send_post_request(req, &context).await?;
                }
                maybe_req = context.from_handler_rx.recv() => {
                    let Some(req) = maybe_req else {
                        return Err(WorkerQuitReason::HandlerTerminated);
                    };
                    http.send_post_request(req, &context).await?;
                }
                maybe_event = sse_stream.next() => {
                    let Some(item) = maybe_event else {
                        return Err(WorkerQuitReason::TransportClosed);
                    };
                    let sse = match item {
                        Ok(sse) => sse,
                        Err(e) => {
                            return Err(WorkerQuitReason::fatal(
                                LegacySseError::Message(format!("SSE stream read error: {e}")),
                                "read SSE event",
                            ));
                        }
                    };
                    let event_name = sse.event.as_deref().unwrap_or("message");
                    if event_name == "endpoint" {
                        if let Some(data) = sse.data.as_deref() {
                            if let Ok(resolved) = LegacySseHttp::resolve_endpoint(&http.server_url, data) {
                                http.post_url = resolved;
                            }
                        }
                        continue;
                    }
                    if matches!(event_name, "" | "message") {
                        let Some(data) = sse.data.as_deref() else {
                            continue;
                        };
                        let trimmed = data.trim();
                        if trimmed.is_empty() || trimmed == "[DONE]" {
                            continue;
                        }
                        if let Ok(msg) = serde_json::from_str::<ServerJsonRpcMessage>(trimmed) {
                            if context.to_handler_tx.send(msg).await.is_err() {
                                return Err(WorkerQuitReason::HandlerTerminated);
                            }
                        }
                    }
                }
            }
        }
    }
}

pub(super) async fn connect_sse(
    core: &Core,
    server_name: &str,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<Arc<dyn McpConnection>, String> {
    let target_url = url.trim().to_string();
    if target_url.is_empty() {
        return Err("SSE server URL cannot be empty".to_string());
    }

    let (custom_headers, manual_bearer) = split_custom_headers(headers)?;
    let client = reqwest::Client::builder()
        .connect_timeout(MCP_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to construct SSE HTTP client: {e}"))?;

    let oauth_bearer = oauth::resolve_bearer_token_if_configured(core, server_name, &target_url).await?;
    let initial_bearer = oauth_bearer.or_else(|| manual_bearer.clone());

    let mut req = client.get(&target_url).header(ACCEPT, "text/event-stream");
    for (name, value) in &custom_headers {
        req = req.header(name, value);
    }
    if let Some(token) = &initial_bearer {
        req = req.header(AUTHORIZATION, format!("Bearer {token}"));
    }

    let resp = timeout(MCP_CONNECT_TIMEOUT, req.send())
        .await
        .map_err(|_| "MCP SSE connect timeout (30s).".to_string())?
        .map_err(|e| format!("MCP SSE connect failed: {e}"))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        oauth::mark_server_auth_required(core, server_name, &target_url);
        return Err("MCP SSE connection unauthorized (401); OAuth sign-in required.".to_string());
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let msg = format!("MCP SSE returned HTTP {}: {}", status.as_u16(), body);
        if is_unauthorized_error_text(&msg) {
            oauth::mark_server_auth_required(core, server_name, &target_url);
        }
        return Err(msg);
    }

    let mut sse_stream: BoxStream<'static, Result<Sse, SseParseError>> =
        SseStream::from_bytes_stream(resp.bytes_stream()).boxed();

    // 等待服务端推送初始 `event: endpoint`，确定后续 JSON-RPC POST 目标地址。
    let post_url = timeout(MCP_CONNECT_TIMEOUT, async {
        while let Some(item) = sse_stream.next().await {
            let sse = item.map_err(|e| format!("MCP SSE handshake read failed: {e}"))?;
            if sse.event.as_deref() == Some("endpoint") {
                if let Some(data) = sse.data.as_deref() {
                    return LegacySseHttp::resolve_endpoint(&target_url, data);
                }
            }
        }
        Err("MCP SSE stream closed before receiving endpoint event".to_string())
    })
    .await
    .map_err(|_| "MCP SSE endpoint handshake timeout (30s).".to_string())??;

    let worker = LegacySseWorker {
        http: LegacySseHttp {
            core: core.clone(),
            server_name: server_name.to_string(),
            server_url: target_url,
            client,
            custom_headers,
            manual_bearer,
            post_url,
        },
        sse_stream,
    };

    let handler = NovaMcpHandler::new(core.clone(), server_name);
    let running = timeout(MCP_CONNECT_TIMEOUT, handler.serve(worker))
        .await
        .map_err(|_| "MCP SSE initialize timeout (30s).".to_string())?
        .map_err(|e| format!("MCP SSE initialize failed: {e}"))?;

    Ok(RmcpConnection::from_running_service(running))
}
