use super::connection::{McpConnection, NovaMcpHandler, RmcpConnection};
use super::{oauth, MCP_CONNECT_TIMEOUT};
use crate::core::Core;
use http::{HeaderName, HeaderValue};
use rmcp::service::ServiceExt;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::timeout;

pub(super) fn split_custom_headers(
    headers: &HashMap<String, String>,
) -> Result<(HashMap<HeaderName, HeaderValue>, Option<String>), String> {
    let mut custom = HashMap::new();
    let mut bearer_auth = None;
    for (k, v) in headers {
        let trimmed_key = k.trim();
        let trimmed_val = v.trim();
        if trimmed_key.is_empty() {
            continue;
        }
        if trimmed_key.eq_ignore_ascii_case("authorization") {
            if let Some(token) = trimmed_val
                .strip_prefix("Bearer ")
                .or_else(|| trimmed_val.strip_prefix("bearer "))
            {
                bearer_auth = Some(token.trim().to_string());
                continue;
            }
        }
        // 保留受 Transport 自动管理的协议头（accept / mcp-session-id / last-event-id）不透传。
        if trimmed_key.eq_ignore_ascii_case("accept")
            || trimmed_key.eq_ignore_ascii_case("mcp-session-id")
            || trimmed_key.eq_ignore_ascii_case("last-event-id")
        {
            continue;
        }
        let name = HeaderName::from_str(trimmed_key)
            .map_err(|e| format!("Invalid header name '{}': {}", k, e))?;
        let value = HeaderValue::from_str(trimmed_val)
            .map_err(|e| format!("Invalid header value for '{}': {}", k, e))?;
        custom.insert(name, value);
    }
    Ok((custom, bearer_auth))
}

pub(super) fn is_unauthorized_error_text(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("401") || lower.contains("unauthorized") || lower.contains("auth required")
}

pub(super) async fn connect_streamable_http(
    core: &Core,
    server_name: &str,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<Arc<dyn McpConnection>, String> {
    let target_url = url.trim().to_string();
    if target_url.is_empty() {
        return Err("Streamable HTTP server URL cannot be empty".to_string());
    }

    let (custom_headers, manual_bearer) = split_custom_headers(headers)?;
    let http_client = reqwest::Client::builder()
        .connect_timeout(MCP_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to construct HTTP client: {e}"))?;

    let mut cfg = StreamableHttpClientTransportConfig::with_uri(target_url.clone())
        .custom_headers(custom_headers);
    let auth_manager = oauth::load_authorized_manager(core, server_name, &target_url).await?;

    if auth_manager.is_none() {
        if let Some(token) = manual_bearer {
            cfg = cfg.auth_header(token);
        }
    }

    let handler = NovaMcpHandler::new(core.clone(), server_name);
    let connect_result = if let Some(manager) = auth_manager {
        let auth_client = oauth::wrap_reqwest_with_auth(http_client, manager);
        let transport = StreamableHttpClientTransport::with_client(auth_client, cfg);
        timeout(MCP_CONNECT_TIMEOUT, handler.serve(transport)).await
    } else {
        let transport = StreamableHttpClientTransport::with_client(http_client, cfg);
        timeout(MCP_CONNECT_TIMEOUT, handler.serve(transport)).await
    };

    match connect_result {
        Err(_) => Err(
            "MCP streamable_http initialize timeout (30s). Remote server may be overloaded or unreachable."
                .to_string(),
        ),
        Ok(Err(e)) => {
            let msg = format!("MCP streamable_http initialize failed: {e}");
            if is_unauthorized_error_text(&msg) {
                oauth::mark_server_auth_required(core, server_name, &target_url);
            }
            Err(msg)
        }
        Ok(Ok(running)) => Ok(RmcpConnection::from_running_service(running)),
    }
}
