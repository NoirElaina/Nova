use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use nova_core::agent::session::history::close_history_pool;
use nova_core::api::mcp::{
    add_mcp_server, call_mcp_tool, get_mcp_server_statuses, list_mcp_resources, list_mcp_tools,
    read_mcp_resource, remove_mcp_server, McpServerConfig,
};
use nova_core::{Core, CoreConfig};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};

// `MCP_RUNTIME` 在 `nova-core` 内为进程级单例，本文件各传输契约测试串行化以避免相互覆盖。
static MCP_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn mcp_test_lock() -> &'static Mutex<()> {
    MCP_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

fn temp_core() -> Result<(TempDir, Core), String> {
    let dir = TempDir::new().map_err(|e| format!("failed to create TempDir: {e}"))?;
    let cfg = CoreConfig::new(dir.path().to_path_buf())
        .with_cache_dir(dir.path().join("cache"))
        .with_download_dir(dir.path().join("downloads"));
    Ok((dir, Core::new(cfg, 256)))
}

fn handle_mock_mcp_rpc(transport: &str, req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {}, "resources": {} },
            "serverInfo": { "name": format!("mock-mcp-{transport}"), "version": "0.1.0" }
        }),
        "tools/list" => json!({
            "tools": [{
                "name": "mock_echo",
                "description": format!("Echo tool over {transport}"),
                "inputSchema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }]
        }),
        "tools/call" => {
            let text = params
                .get("arguments")
                .and_then(|a| a.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            json!({
                "content": [{ "type": "text", "text": format!("{transport}:{text}") }]
            })
        }
        "resources/list" => json!({
            "resources": [{
                "uri": format!("mock://{transport}/info"),
                "name": format!("{transport}-info"),
                "mimeType": "text/plain"
            }]
        }),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            json!({
                "contents": [{ "uri": uri, "text": format!("{transport}-resource-body") }]
            })
        }
        other => {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("unknown method: {other}") }
            });
        }
    };

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

async fn verify_mcp_contract(core: &Core, name: &str, expected_prefix: &str) -> Result<(), String> {
    let statuses = get_mcp_server_statuses(core).await?;
    let found = statuses
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("server {name} missing from status list"))?;
    assert_eq!(found.status, "connected", "server status={:?}", found);

    let tools = list_mcp_tools(core, name.to_string()).await?;
    assert!(tools.iter().any(|t| t.name == "mock_echo"));

    let call_res = call_mcp_tool(
        core,
        name.to_string(),
        "mock_echo".to_string(),
        json!({ "text": "hello" }),
    )
    .await?;
    let call_text = serde_json::to_string(&call_res)
        .map_err(|e| format!("serialize tool result: {e}"))?;
    assert!(
        call_text.contains(&format!("{expected_prefix}:hello")),
        "unexpected tool output: {call_text}"
    );

    let resources = list_mcp_resources(core, name.to_string()).await?;
    assert!(!resources.is_empty());

    let res_body = read_mcp_resource(core, name.to_string(), resources[0].uri.clone()).await?;
    let body_str = serde_json::to_string(&res_body)
        .map_err(|e| format!("serialize resource: {e}"))?;
    assert!(body_str.contains(&format!("{expected_prefix}-resource-body")));
    Ok(())
}

#[tokio::test]
async fn mcp_stdio_transport_black_box_contract() {
    let _guard = mcp_test_lock().lock().await;
    let (_dir, core) = temp_core().expect("temp core");

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mock_mcp_stdio.mjs");

    let server_name = "mock-stdio-contract";
    add_mcp_server(
        &core,
        server_name.to_string(),
        McpServerConfig::Stdio {
            command: "node".to_string(),
            args: vec![fixture.to_string_lossy().into_owned()],
            env: HashMap::new(),
        },
        None,
    )
    .await
    .expect("add stdio mcp server");

    verify_mcp_contract(&core, server_name, "stdio")
        .await
        .expect("verify stdio mcp contract");

    remove_mcp_server(&core, server_name.to_string())
        .await
        .expect("remove stdio mcp server");
    close_history_pool(&core).await;
}

#[tokio::test]
async fn mcp_sse_transport_black_box_contract() {
    use futures_util::StreamExt;

    let _guard = mcp_test_lock().lock().await;
    let (_dir, core) = temp_core().expect("temp core");

    let (tx, _) = broadcast::channel::<String>(32);
    let tx_clone = tx.clone();

    let sse_handler = {
        let tx = tx.clone();
        move || {
            let rx = tx.subscribe();
            async move {
                let first = futures_util::stream::once(async {
                    Ok::<Event, std::convert::Infallible>(
                        Event::default().event("endpoint").data("/message"),
                    )
                });
                let rest = futures_util::stream::unfold(rx, |mut r| async move {
                    loop {
                        match r.recv().await {
                            Ok(payload) => {
                                return Some((
                                    Ok::<Event, std::convert::Infallible>(
                                        Event::default().event("message").data(payload),
                                    ),
                                    r,
                                ));
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                });
                Sse::new(first.chain(rest)).keep_alive(KeepAlive::default())
            }
        }
    };

    let post_handler = move |axum::Json(req): axum::Json<Value>| {
        let tx = tx_clone.clone();
        async move {
            if req.get("id").is_some() {
                let reply = handle_mock_mcp_rpc("sse", &req);
                let _ = tx.send(reply.to_string());
            }
            StatusCode::ACCEPTED
        }
    };

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(post_handler));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock sse mcp");
    let addr = listener.local_addr().expect("addr");
    let server_task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let server_name = "mock-sse-contract";
    add_mcp_server(
        &core,
        server_name.to_string(),
        McpServerConfig::Sse {
            url: format!("http://{addr}/sse"),
            headers: HashMap::new(),
        },
        None,
    )
    .await
    .expect("add sse mcp server");

    verify_mcp_contract(&core, server_name, "sse")
        .await
        .expect("verify sse mcp contract");

    remove_mcp_server(&core, server_name.to_string())
        .await
        .expect("remove sse mcp server");
    server_task.abort();
    close_history_pool(&core).await;
}

#[tokio::test]
async fn mcp_streamable_http_transport_black_box_contract() {
    let _guard = mcp_test_lock().lock().await;
    let (_dir, core) = temp_core().expect("temp core");

    let app = Router::new().route(
        "/mcp",
        post(|_headers: HeaderMap, axum::Json(req): axum::Json<Value>| async move {
            if req.get("id").is_none() {
                return (
                    StatusCode::ACCEPTED,
                    [("mcp-session-id", "mock-session-1")],
                    axum::Json(json!({})),
                )
                    .into_response();
            }
            let reply = handle_mock_mcp_rpc("streamable_http", &req);
            (
                StatusCode::OK,
                [("mcp-session-id", "mock-session-1")],
                axum::Json(reply),
            )
                .into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock streamable_http mcp");
    let addr = listener.local_addr().expect("addr");
    let server_task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let server_name = "mock-streamable-http-contract";
    add_mcp_server(
        &core,
        server_name.to_string(),
        McpServerConfig::StreamableHttp {
            url: format!("http://{addr}/mcp"),
            headers: HashMap::new(),
        },
        None,
    )
    .await
    .expect("add streamable_http mcp server");

    verify_mcp_contract(&core, server_name, "streamable_http")
        .await
        .expect("verify streamable_http mcp contract");

    remove_mcp_server(&core, server_name.to_string())
        .await
        .expect("remove streamable_http mcp server");
    server_task.abort();
    close_history_pool(&core).await;
}
