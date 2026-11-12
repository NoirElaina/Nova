use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use futures_util::{Stream, StreamExt};
use reqwest::header::{HeaderMap, ACCEPT, CONTENT_TYPE};
use serde_json::{json, Value};
use tokio::sync::{oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::streamable_http::build_header_map;
use super::{McpResourceInfo, McpToolInfo, MCP_CONNECT_TIMEOUT};

type ResponseMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;
type BoxByteStream =
    Pin<Box<dyn Stream<Item = Result<tokio_util::bytes::Bytes, reqwest::Error>> + Send>>;

const SSE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SSE_PENDING_RESPONSES: usize = 128;

fn insert_pending_response(
    map: &mut HashMap<u64, oneshot::Sender<Value>>,
    id: u64,
    tx: oneshot::Sender<Value>,
    method: &str,
) -> Result<(), String> {
    if map.len() >= MAX_SSE_PENDING_RESPONSES {
        return Err(format!(
            "MCP SSE pending request limit ({}) reached; rejecting request '{}'",
            MAX_SSE_PENDING_RESPONSES, method
        ));
    }
    map.insert(id, tx);
    Ok(())
}

pub(super) struct SseMcpConnection {
    client: reqwest::Client,
    #[allow(dead_code)]
    sse_url: String,
    /// 消息投递端点。SSE 断线重连后服务端可能下发新的 endpoint，因此必须共享可变。
    endpoint: Arc<tokio::sync::RwLock<String>>,
    headers: HeaderMap,
    next_id: Arc<AtomicU64>,
    response_txs: ResponseMap,
    ready_rx: watch::Receiver<bool>,
    cancel_token: CancellationToken,
}

fn initialize_params() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "roots": { "listChanged": true }
        },
        "clientInfo": {
            "name": "nova",
            "version": "0.1.0"
        }
    })
}

fn with_custom_headers(
    headers: &HeaderMap,
    mut request_builder: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    for (name, value) in headers.iter() {
        request_builder = request_builder.header(name, value);
    }
    request_builder
}

fn parse_jsonrpc_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
    })
}

fn compact_body_for_error(raw: &str) -> String {
    let mut compact = raw.replace(['\n', '\r'], " ");
    if compact.len() > 240 {
        compact.truncate(240);
        compact.push_str("...");
    }
    compact
}

struct SseRpcContext<'a> {
    client: &'a reqwest::Client,
    headers: &'a HeaderMap,
    endpoint_lock: &'a Arc<tokio::sync::RwLock<String>>,
    next_id: &'a AtomicU64,
    response_txs: &'a ResponseMap,
}

async fn post_rpc_request(
    ctx: SseRpcContext<'_>,
    method: &str,
    params: Value,
    req_timeout: Duration,
) -> Result<Value, String> {
    let SseRpcContext {
        client,
        headers,
        endpoint_lock,
        next_id,
        response_txs,
    } = ctx;
    let id = next_id.fetch_add(1, Ordering::Relaxed);

    let (tx, rx) = oneshot::channel();
    {
        let mut map = response_txs.lock().await;
        insert_pending_response(&mut map, id, tx, method)?;
    }

    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });

    let endpoint = endpoint_lock.read().await.clone();
    let request_builder = with_custom_headers(
        headers,
        client
            .post(&endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream"),
    )
    .json(&req);

    let resp = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            response_txs.lock().await.remove(&id);
            return Err(format!("MCP SSE POST request failed: {}", e));
        }
    };

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        response_txs.lock().await.remove(&id);
        return Err(format!(
            "MCP SSE POST HTTP {}: {}",
            status,
            compact_body_for_error(&body)
        ));
    }

    // 某些 MCP 服务会直接在 POST 响应体中同步返回 JSON-RPC 结果
    let trimmed = body.trim();
    if !trimmed.is_empty() {
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            if parse_jsonrpc_id(&val) == Some(id) || val.get("result").is_some() || val.get("error").is_some() {
                response_txs.lock().await.remove(&id);
                if let Some(err) = val.get("error") {
                    return Err(format!("MCP error: {}", err));
                }
                return Ok(val.get("result").cloned().unwrap_or_else(|| json!({})));
            }
        }
    }

    // 否则等待 SSE 事件流推送对应响应。
    // 时限必须与 `call_mcp_tool` 的 60s 预算同量级：用 30s 的连接超时
    // 会把 30-60s 的正常工具调用提前判死。
    match timeout(req_timeout, rx).await {
        Ok(Ok(val)) => {
            if let Some(err) = val.get("error") {
                Err(format!("MCP error: {}", err))
            } else {
                Ok(val.get("result").cloned().unwrap_or_else(|| json!({})))
            }
        }
        Ok(Err(_)) => {
            Err("MCP SSE channel dropped without response (server disconnected)".to_string())
        }
        Err(_) => {
            response_txs.lock().await.remove(&id);
            Err(format!("MCP SSE request id {} timed out waiting for response", id))
        }
    }
}

async fn post_rpc_notification(
    client: &reqwest::Client,
    headers: &HeaderMap,
    endpoint_lock: &Arc<tokio::sync::RwLock<String>>,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let req = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });

    let endpoint = endpoint_lock.read().await.clone();
    let request_builder = with_custom_headers(
        headers,
        client
            .post(&endpoint)
            .header(CONTENT_TYPE, "application/json"),
    )
    .json(&req);

    let resp = request_builder
        .send()
        .await
        .map_err(|e| format!("MCP SSE notification failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "MCP SSE notification HTTP {}: {}",
            status,
            compact_body_for_error(&body)
        ));
    }

    Ok(())
}

impl SseMcpConnection {
    async fn wait_until_ready(&mut self) -> Result<(), String> {
        if *self.ready_rx.borrow() {
            return Ok(());
        }
        let wait_fut = async {
            loop {
                if self.cancel_token.is_cancelled() {
                    return Err("MCP SSE connection cancelled".to_string());
                }
                if *self.ready_rx.borrow() {
                    return Ok(());
                }
                tokio::select! {
                    _ = self.cancel_token.cancelled() => {
                        return Err("MCP SSE connection cancelled".to_string());
                    }
                    res = self.ready_rx.changed() => {
                        if res.is_err() {
                            return Err("MCP SSE connection closed".to_string());
                        }
                    }
                }
            }
        };
        timeout(MCP_CONNECT_TIMEOUT, wait_fut)
            .await
            .map_err(|_| "MCP SSE reconnecting timeout waiting for session endpoint/initialize".to_string())?
    }

    pub(super) async fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.wait_until_ready().await?;
        post_rpc_request(
            SseRpcContext {
                client: &self.client,
                headers: &self.headers,
                endpoint_lock: &self.endpoint,
                next_id: &self.next_id,
                response_txs: &self.response_txs,
            },
            method,
            params,
            SSE_REQUEST_TIMEOUT,
        )
        .await
    }

    pub(super) async fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.wait_until_ready().await?;
        post_rpc_notification(&self.client, &self.headers, &self.endpoint, method, params).await
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
        self.send_request("resources/read", json!({ "uri": uri })).await
    }

    pub(super) async fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        self.send_request(
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments
            }),
        )
        .await
    }

    pub(super) async fn shutdown(&mut self) {
        self.cancel_token.cancel();
        self.response_txs.lock().await.clear();
    }
}

/// 解析服务端下发的 endpoint（可能是绝对 URL，也可能是相对路径）。
fn resolve_message_endpoint(base_url: &Url, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(raw) {
        return Some(parsed.to_string());
    }
    base_url.join(raw).ok().map(|u| u.to_string())
}

fn push_sse_chunk(buffer: &mut String, bytes: &[u8]) {
    buffer.push_str(&String::from_utf8_lossy(bytes));
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n");
    }
}

fn parse_sse_block(block: &str) -> (String, String) {
    let mut event_type = "message".to_string();
    let mut data_lines: Vec<String> = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim().to_string());
        }
    }
    (event_type, data_lines.join("\n"))
}

enum SseEventOutcome {
    Endpoint,
    Other,
}

/// 解析单个 SSE 事件块：`endpoint` 事件更新投递端点，`message` 事件唤醒等待方。
async fn handle_sse_block(
    block: &str,
    base_url: &Url,
    response_txs: &ResponseMap,
    endpoint: &Arc<tokio::sync::RwLock<String>>,
) -> SseEventOutcome {
    let (event_type, data) = parse_sse_block(block);
    match event_type.as_str() {
        "endpoint" => {
            if let Some(resolved) = resolve_message_endpoint(base_url, &data) {
                *endpoint.write().await = resolved;
                return SseEventOutcome::Endpoint;
            }
            SseEventOutcome::Other
        }
        "message" => {
            if let Ok(val) = serde_json::from_str::<Value>(&data) {
                if let Some(id) = parse_jsonrpc_id(&val) {
                    if let Some(tx) = response_txs.lock().await.remove(&id) {
                        let _ = tx.send(val);
                    }
                }
            }
            SseEventOutcome::Other
        }
        _ => SseEventOutcome::Other,
    }
}

async fn run_reinitialize(
    client: reqwest::Client,
    headers: HeaderMap,
    endpoint: Arc<tokio::sync::RwLock<String>>,
    next_id: Arc<AtomicU64>,
    response_txs: ResponseMap,
) -> Result<(), String> {
    timeout(
        MCP_CONNECT_TIMEOUT,
        post_rpc_request(
            SseRpcContext {
                client: &client,
                headers: &headers,
                endpoint_lock: &endpoint,
                next_id: &next_id,
                response_txs: &response_txs,
            },
            "initialize",
            initialize_params(),
            MCP_CONNECT_TIMEOUT,
        ),
    )
    .await
    .map_err(|_| "MCP SSE reconnect initialize timeout (30s).".to_string())??;

    let _ = post_rpc_notification(
        &client,
        &headers,
        &endpoint,
        "notifications/initialized",
        json!({}),
    )
    .await;
    Ok(())
}

/// 建立 SSE MCP 连接：
/// 1. 发起 GET 握手并读取初始事件流以获取 endpoint URL。
/// 2. 将同一条 GET 流直接交给后台任务消费后续事件（断流后再重连并重新 initialize）。
/// 3. 执行 initialize 握手与 initialized 通知。
pub(super) async fn connect_sse(
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<SseMcpConnection, String> {
    let header_map = build_header_map(headers)?;
    // 不能用 `timeout()`：那是"从连接开始到 body 流读完"的**总**超时，
    // 会把长连接 SSE 在 60s 处直接掐断。这里只约束连接与静默读。
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("Failed to build HTTP client for MCP SSE: {}", e))?;

    let mut req = client.get(url).header(ACCEPT, "text/event-stream");
    for (name, value) in header_map.iter() {
        req = req.header(name, value);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Failed to initiate SSE connection to {}: {}", url, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "MCP SSE initial GET returned HTTP {}: {}",
            status,
            compact_body_for_error(&body)
        ));
    }

    let base_url = Url::parse(url).map_err(|e| format!("Invalid MCP SSE url '{}': {}", url, e))?;

    let mut byte_stream: BoxByteStream = Box::pin(resp.bytes_stream());
    let mut buffer = String::new();
    let mut message_endpoint_url: Option<String> = None;
    let mut initial_eof = false;

    // 读取初始握手流，直到捕获 endpoint 事件
    let handshake_deadline = tokio::time::Instant::now() + MCP_CONNECT_TIMEOUT;
    while tokio::time::Instant::now() < handshake_deadline {
        let chunk_opt = match timeout(Duration::from_millis(1500), byte_stream.next()).await {
            Ok(Some(chunk_res)) => {
                Some(chunk_res.map_err(|e| format!("SSE stream read error: {}", e))?)
            }
            Ok(None) => {
                initial_eof = true;
                break;
            }
            Err(_) => None,
        };

        if let Some(chunk) = chunk_opt {
            push_sse_chunk(&mut buffer, &chunk);
        }

        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer.drain(..pos + 2);

            let (event_type, data) = parse_sse_block(&event_block);
            if event_type == "endpoint" {
                let resolved = resolve_message_endpoint(&base_url, &data).ok_or_else(|| {
                    format!("Failed to resolve relative endpoint '{}'", data)
                })?;
                message_endpoint_url = Some(resolved);
                break;
            }
        }

        if message_endpoint_url.is_some() {
            break;
        }
    }

    let endpoint = message_endpoint_url.unwrap_or_else(|| {
        // 如果服务器没有发送明确的 endpoint 事件，按 MCP 缺省规范 POST 到基础 URL
        url.to_string()
    });

    let response_txs: ResponseMap = Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(AtomicU64::new(1));
    let cancel_token = CancellationToken::new();
    let endpoint_shared: Arc<tokio::sync::RwLock<String>> =
        Arc::new(tokio::sync::RwLock::new(endpoint.clone()));
    let (ready_tx, ready_rx) = watch::channel(true);

    // 保留初始 GET 流继续在后台读：只有当初始流断开时才触发重连 GET + re-initialize。
    let mut pending_initial: Option<(BoxByteStream, String, bool)> =
        Some((byte_stream, buffer, initial_eof));

    {
        let response_txs = Arc::clone(&response_txs);
        let cancel_token = cancel_token.clone();
        let endpoint_shared = Arc::clone(&endpoint_shared);
        let client = client.clone();
        let url = url.to_string();
        let header_map = header_map.clone();
        let next_id = Arc::clone(&next_id);
        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(250);
            loop {
                if cancel_token.is_cancelled() {
                    break;
                }

                let (mut stream, mut stream_buffer, is_reconnect, early_eof) =
                    if let Some((init_stream, init_buf, eof)) = pending_initial.take() {
                        (init_stream, init_buf, false, eof)
                    } else {
                        let _ = ready_tx.send(false);
                        let mut req = client.get(&url).header(ACCEPT, "text/event-stream");
                        for (name, value) in header_map.iter() {
                            req = req.header(name, value);
                        }

                        let connected = tokio::select! {
                            _ = cancel_token.cancelled() => break,
                            res = req.send() => match res {
                                Ok(resp) if resp.status().is_success() => Some(resp),
                                Ok(resp) => {
                                    tracing::warn!(
                                        status = %resp.status(),
                                        "MCP SSE reconnect rejected; will retry"
                                    );
                                    None
                                }
                                Err(err) => {
                                    tracing::warn!(error = %err, "MCP SSE reconnect failed; will retry");
                                    None
                                }
                            }
                        };

                        if let Some(resp) = connected {
                            let s: BoxByteStream = Box::pin(resp.bytes_stream());
                            (s, String::new(), true, false)
                        } else {
                            response_txs.lock().await.clear();
                            if cancel_token.is_cancelled() {
                                break;
                            }
                            tokio::select! {
                                _ = cancel_token.cancelled() => break,
                                _ = tokio::time::sleep(backoff) => {}
                            }
                            backoff = (backoff * 2).min(Duration::from_secs(10));
                            continue;
                        }
                    };

                let mut reinit_rx: Option<oneshot::Receiver<Result<(), String>>> = None;
                let mut reinit_task: Option<JoinHandle<()>> = None;
                let mut reinit_started = false;

                // 先排空握手期间同包到达但未消费的完整事件块。
                while let Some(pos) = stream_buffer.find("\n\n") {
                    let block = stream_buffer[..pos].to_string();
                    stream_buffer.drain(..pos + 2);
                    handle_sse_block(&block, &base_url, &response_txs, &endpoint_shared).await;
                }

                if !early_eof {
                    loop {
                        let chunk = tokio::select! {
                            _ = cancel_token.cancelled() => {
                                if let Some(h) = reinit_task.take() {
                                    h.abort();
                                }
                                response_txs.lock().await.clear();
                                return;
                            }
                            reinit_res = async {
                                match reinit_rx.as_mut() {
                                    Some(rx) => rx.await,
                                    None => std::future::pending().await,
                                }
                            }, if reinit_rx.is_some() => {
                                reinit_rx = None;
                                match reinit_res {
                                    Ok(Ok(())) => {
                                        backoff = Duration::from_millis(250);
                                        let _ = ready_tx.send(true);
                                    }
                                    Ok(Err(err)) => {
                                        tracing::warn!(error = %err, "MCP SSE reconnect initialize failed; reconnecting");
                                        break;
                                    }
                                    Err(_) => {
                                        tracing::warn!("MCP SSE reconnect initialize aborted; reconnecting");
                                        break;
                                    }
                                }
                                continue;
                            }
                            chunk = stream.next() => chunk,
                        };

                        match chunk {
                            Some(Ok(bytes)) => {
                                if !is_reconnect {
                                    backoff = Duration::from_millis(250);
                                }
                                push_sse_chunk(&mut stream_buffer, &bytes);
                                while let Some(pos) = stream_buffer.find("\n\n") {
                                    let block = stream_buffer[..pos].to_string();
                                    stream_buffer.drain(..pos + 2);
                                    let outcome = handle_sse_block(
                                        &block,
                                        &base_url,
                                        &response_txs,
                                        &endpoint_shared,
                                    )
                                    .await;
                                    if is_reconnect && !reinit_started
                                        && matches!(outcome, SseEventOutcome::Endpoint) {
                                            reinit_started = true;
                                            let (tx, rx) = oneshot::channel();
                                            reinit_rx = Some(rx);
                                            let c = client.clone();
                                            let h = header_map.clone();
                                            let ep = Arc::clone(&endpoint_shared);
                                            let nid = Arc::clone(&next_id);
                                            let txs = Arc::clone(&response_txs);
                                            reinit_task = Some(tokio::spawn(async move {
                                                let res = run_reinitialize(c, h, ep, nid, txs).await;
                                                let _ = tx.send(res);
                                            }));
                                        }
                                }
                            }
                            _ => break,
                        }
                    }
                }

                if let Some(h) = reinit_task.take() {
                    h.abort();
                }
                let _ = ready_tx.send(false);
                // 断流：让所有在途请求立刻拿到错误，而不是各自等 120s 超时。
                response_txs.lock().await.clear();
                if cancel_token.is_cancelled() {
                    break;
                }
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        });
    }

    let mut conn = SseMcpConnection {
        client,
        sse_url: url.to_string(),
        endpoint: endpoint_shared,
        headers: header_map,
        next_id,
        response_txs,
        ready_rx,
        cancel_token,
    };

    let init_result = timeout(
        MCP_CONNECT_TIMEOUT,
        conn.send_request("initialize", initialize_params()),
    )
    .await;

    match init_result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            conn.shutdown().await;
            return Err(e);
        }
        Err(_) => {
            conn.shutdown().await;
            return Err("MCP SSE initialize timeout (30s).".to_string());
        }
    }

    let _ = conn
        .send_notification("notifications/initialized", json!({}))
        .await;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Option<(String, String)> {
        let mut buf = Vec::new();
        let mut temp = [0u8; 4096];
        let mut header_end = None;
        let mut content_len = 0usize;

        loop {
            let n = stream.read(&mut temp).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&temp[..n]);
            if header_end.is_none() {
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buf[..pos + 4]);
                    for line in headers.lines() {
                        if let Some((k, v)) = line.split_once(':') {
                            if k.trim().eq_ignore_ascii_case("content-length") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
            }
            if let Some(end) = header_end {
                if buf.len() >= end + content_len {
                    let head = String::from_utf8_lossy(&buf[..end]).to_string();
                    let body = String::from_utf8_lossy(&buf[end..end + content_len]).to_string();
                    return Some((head, body));
                }
            }
        }
    }

    #[tokio::test]
    async fn connect_sse_reuses_handshake_stream_and_reinitializes_on_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let get_count = Arc::new(AtomicUsize::new(0));
        let init_count = Arc::new(AtomicUsize::new(0));
        let seen_methods = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let (drop_sse1_tx, drop_sse1_rx) = oneshot::channel::<()>();
        let drop_sse1_rx = Arc::new(Mutex::new(Some(drop_sse1_rx)));
        let sse_writers: Arc<Mutex<HashMap<usize, tokio::sync::mpsc::UnboundedSender<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let get_count_srv = Arc::clone(&get_count);
        let init_count_srv = Arc::clone(&init_count);
        let seen_methods_srv = Arc::clone(&seen_methods);
        let drop_rx_holder = Arc::clone(&drop_sse1_rx);
        let writers_srv = Arc::clone(&sse_writers);

        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let get_count = Arc::clone(&get_count_srv);
                let init_count = Arc::clone(&init_count_srv);
                let seen_methods = Arc::clone(&seen_methods_srv);
                let drop_rx_holder = Arc::clone(&drop_rx_holder);
                let writers = Arc::clone(&writers_srv);

                tokio::spawn(async move {
                    let Some((head, body)) = read_http_request(&mut socket).await else {
                        return;
                    };
                    let first_line = head.lines().next().unwrap_or("");
                    if first_line.starts_with("GET /sse ") {
                        let session = get_count.fetch_add(1, Ordering::SeqCst) + 1;
                        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                        writers.lock().await.insert(session, tx);

                        let init_payload = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\nevent: endpoint\r\ndata: /messages?session={}\r\n\r\n",
                            session
                        );
                        let _ = socket.write_all(init_payload.as_bytes()).await;

                        let mut drop_rx = if session == 1 {
                            drop_rx_holder.lock().await.take()
                        } else {
                            None
                        };

                        loop {
                            tokio::select! {
                                msg = rx.recv() => match msg {
                                    Some(payload) => {
                                        if socket.write_all(payload.as_bytes()).await.is_err() {
                                            break;
                                        }
                                    }
                                    None => break,
                                },
                                _ = async {
                                    if let Some(ref mut drx) = drop_rx {
                                        let _ = drx.await;
                                    } else {
                                        std::future::pending::<()>().await;
                                    }
                                } => {
                                    let _ = socket.shutdown().await;
                                    break;
                                }
                            }
                        }
                    } else if first_line.starts_with("POST /messages?session=") {
                        let path = first_line.split_whitespace().nth(1).unwrap_or("").to_string();
                        let session: usize = path
                            .split("session=")
                            .nth(1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                        let method = msg
                            .get("method")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if method == "initialize" {
                            init_count.fetch_add(1, Ordering::SeqCst);
                        }
                        seen_methods
                            .lock()
                            .await
                            .push((path.clone(), method.clone()));

                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;

                        if let Some(id) = msg.get("id").cloned() {
                            if method == "tools/call" {
                                // 故意挂起 session 1 的工具调用直到服务端断开 SSE1
                                return;
                            }
                            let result = match method.as_str() {
                                "initialize" => json!({ "protocolVersion": "2024-11-05" }),
                                "tools/list" => json!({ "tools": [{ "name": format!("tool_s{}", session) }] }),
                                _ => json!({}),
                            };
                            let rpc_resp = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": result,
                            });
                            let sse_ev = format!("event: message\ndata: {}\n\n", rpc_resp);
                            if let Some(tx) = writers.lock().await.get(&session) {
                                let _ = tx.send(sse_ev);
                            }
                        }
                    }
                });
            }
        });

        let url = format!("http://{}/sse", addr);
        let mut conn = connect_sse(&url, &HashMap::new())
            .await
            .expect("initial SSE connect should succeed");

        // 1. 初始握手只能发一次 GET，不得丢掉握手流重连第二条
        assert_eq!(get_count.load(Ordering::SeqCst), 1);
        assert_eq!(init_count.load(Ordering::SeqCst), 1);

        let tools1 = conn.list_tools().await.expect("list_tools on session 1");
        assert_eq!(tools1[0].name, "tool_s1");
        assert_eq!(get_count.load(Ordering::SeqCst), 1);

        // 2. 在途请求期间服务端断流：在途请求立刻失败，随后重连并自动 re-initialize 新会话
        let _ = drop_sse1_tx.send(());
        let inflight_err = conn.call_tool("slow", json!({})).await;
        assert!(inflight_err.is_err(), "disconnected in-flight request must fail");

        let tools2 = conn
            .list_tools()
            .await
            .expect("list_tools should succeed after reconnect + re-initialize");
        assert_eq!(tools2[0].name, "tool_s2");
        assert!(get_count.load(Ordering::SeqCst) >= 2);
        assert_eq!(init_count.load(Ordering::SeqCst), 2);

        let log = seen_methods.lock().await.clone();
        let session2_methods: Vec<&str> = log
            .iter()
            .filter(|(p, _)| p == "/messages?session=2")
            .map(|(_, m)| m.as_str())
            .collect();
        assert_eq!(
            session2_methods,
            vec!["initialize", "notifications/initialized", "tools/list"]
        );

        conn.shutdown().await;
        server.abort();
    }

    #[test]
    fn pending_response_map_rejects_overflow_beyond_cap() {
        let mut map = HashMap::new();
        for i in 0..MAX_SSE_PENDING_RESPONSES as u64 {
            let (tx, _rx) = oneshot::channel();
            insert_pending_response(&mut map, i, tx, "tools/call")
                .expect("entries below cap should succeed");
        }
        let (tx, _rx) = oneshot::channel();
        let err = insert_pending_response(
            &mut map,
            MAX_SSE_PENDING_RESPONSES as u64,
            tx,
            "tools/call",
        )
        .expect_err("entry at cap must be rejected");
        assert!(err.contains("pending request limit"));
    }
}
