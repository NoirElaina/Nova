use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{json, Value};
use tokio::time::timeout;

use super::{McpResourceInfo, McpToolInfo, MCP_CONNECT_TIMEOUT};

pub(super) struct StreamableHttpMcpConnection {
    client: reqwest::Client,
    url: String,
    session_id: Option<String>,
    next_id: u64,
}

fn extract_session_id_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn parse_jsonrpc_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
    })
}

fn collect_sse_json_messages(body: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut data_lines: Vec<String> = Vec::new();

    let flush_event = |lines: &mut Vec<String>, target: &mut Vec<Value>| {
        if lines.is_empty() {
            return;
        }

        let payload = lines.join("\n");
        lines.clear();

        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return;
        }

        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            target.push(v);
        }
    };

    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            flush_event(&mut data_lines, &mut messages);
            continue;
        }

        if let Some(rest) = line.strip_prefix("data:") {
            // 兼容 data: 和 data: 两种前缀形式。
            let payload = rest.strip_prefix(' ').unwrap_or(rest);
            data_lines.push(payload.to_string());
        }
    }

    flush_event(&mut data_lines, &mut messages);
    messages
}

fn collect_mcp_json_messages(body: &str) -> Vec<Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if let Some(arr) = v.as_array() {
            return arr.clone();
        }
        return vec![v];
    }

    collect_sse_json_messages(trimmed)
}

fn compact_body_for_error(raw: &str) -> String {
    let mut compact = raw.replace('\n', " ").replace('\r', " ");
    if compact.len() > 240 {
        compact.truncate(240);
        compact.push_str("...");
    }
    compact
}

fn parse_mcp_response_for_id(body: &str, request_id: u64) -> Result<Value, String> {
    let messages = collect_mcp_json_messages(body);
    if messages.is_empty() {
        return Err("MCP streamable_http returned empty or invalid response body".to_string());
    }

    for msg in &messages {
        let msg_id = parse_jsonrpc_id(msg);
        if msg_id == Some(request_id) {
            return Ok(msg.clone());
        }
    }

    for msg in &messages {
        if parse_jsonrpc_id(msg).is_none()
            && (msg.get("result").is_some() || msg.get("error").is_some())
        {
            return Ok(msg.clone());
        }
    }

    Err(format!(
        "MCP streamable_http did not return response for request id {}. body={} ",
        request_id,
        compact_body_for_error(body)
    ))
}

impl StreamableHttpMcpConnection {
    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let mut request_builder = self
            .client
            .post(&self.url)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&req);

        if let Some(session_id) = self.session_id.as_deref() {
            request_builder = request_builder.header("mcp-session-id", session_id);
        }

        let resp = request_builder
            .send()
            .await
            .map_err(|e| format!("MCP streamable_http request failed: {}", e))?;

        let status = resp.status();
        if let Some(session_id) = extract_session_id_from_headers(resp.headers()) {
            self.session_id = Some(session_id);
        }
        let body = resp
            .text()
            .await
            .map_err(|e| format!("MCP streamable_http read body failed: {}", e))?;

        if !status.is_success() {
            return Err(format!(
                "MCP streamable_http HTTP {}: {}",
                status,
                compact_body_for_error(&body)
            ));
        }

        let msg = parse_mcp_response_for_id(&body, id)?;
        if let Some(err) = msg.get("error") {
            return Err(format!("MCP error: {}", err));
        }

        Ok(msg.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let mut request_builder = self
            .client
            .post(&self.url)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&req);

        if let Some(session_id) = self.session_id.as_deref() {
            request_builder = request_builder.header("mcp-session-id", session_id);
        }

        let resp = request_builder
            .send()
            .await
            .map_err(|e| format!("MCP streamable_http notification failed: {}", e))?;

        if let Some(session_id) = extract_session_id_from_headers(resp.headers()) {
            self.session_id = Some(session_id);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "MCP streamable_http notification HTTP {}: {}",
                status,
                compact_body_for_error(&body)
            ));
        }

        Ok(())
    }

    pub(super) async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, String> {
        let result = self.send_request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(|v| v.as_str())?.to_string();
                let description = t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let input_schema = t.get("inputSchema").cloned();
                Some(McpToolInfo {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect())
    }

    pub(super) async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        self.send_request(
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments
            }),
        )
        .await
    }

    pub(super) async fn list_resources(&mut self) -> Result<Vec<McpResourceInfo>, String> {
        let result = self.send_request("resources/list", json!({})).await?;
        let resources = result
            .get("resources")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(resources
            .into_iter()
            .filter_map(|r| {
                let uri = r.get("uri").and_then(|v| v.as_str())?.to_string();
                let name = r
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&uri)
                    .to_string();
                let description = r
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let mime_type = r
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(McpResourceInfo {
                    uri,
                    name,
                    description,
                    mime_type,
                })
            })
            .collect())
    }

    pub(super) async fn read_resource(&mut self, uri: &str) -> Result<Value, String> {
        self.send_request("resources/read", json!({ "uri": uri }))
            .await
    }

    pub(super) async fn shutdown(&mut self) {
        // streamable_http 为无状态连接，无需显式关闭。
    }
}

pub(super) async fn connect_streamable_http(
    url: &str,
) -> Result<StreamableHttpMcpConnection, String> {
    let client = reqwest::Client::builder()
        .timeout(MCP_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client for MCP streamable_http: {}", e))?;

    let mut conn = StreamableHttpMcpConnection {
        client,
        url: url.to_string(),
        session_id: None,
        next_id: 1,
    };

    let init_result = timeout(
        MCP_CONNECT_TIMEOUT,
        conn.send_request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "nova",
                    "version": "0.1.0"
                }
            }),
        ),
    )
    .await;

    match init_result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err("MCP streamable_http initialize timeout (30s).".to_string());
        }
    }

    let _ = conn
        .send_notification("notifications/initialized", json!({}))
        .await;

    Ok(conn)
}
