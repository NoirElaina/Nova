use std::process::Stdio;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use nova_protocol::settings::{AppSettings, ProviderProfile};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;

async fn read_rpc_response(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    expected_id: i64,
    events: &mut Vec<Value>,
) -> Result<Value, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!("timed out waiting for rpc id={expected_id}"));
        }
        let next = tokio::time::timeout(remaining, lines.next_line())
            .await
            .map_err(|_| format!("timeout reading stdout for rpc id={expected_id}"))?
            .map_err(|e| format!("stdout read io: {e}"))?
            .ok_or_else(|| "unexpected EOF from nova serve".to_string())?;
        let parsed: Value = serde_json::from_str(&next)
            .map_err(|e| format!("invalid jsonl line: {e}: {next}"))?;
        if parsed.get("id").and_then(Value::as_i64) == Some(expected_id) {
            return Ok(parsed);
        }
        events.push(parsed);
    }
}

async fn rpc_call(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    events: &mut Vec<Value>,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    stdin
        .write_all(format!("{req}\n").as_bytes())
        .await
        .unwrap_or_else(|e| panic!("write {method}: {e}"));
    read_rpc_response(lines, id, events)
        .await
        .unwrap_or_else(|e| panic!("rpc response {method}: {e}"))
}

async fn start_delayed_mock_openai_server() -> Result<(String, tokio::task::JoinHandle<()>), String> {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async move {
            let stream = futures_util::stream::unfold(0u8, |step| async move {
                match step {
                    0 => {
                        let chunk = json!({
                            "id": "chatcmpl_serve",
                            "choices": [{
                                "index": 0,
                                "delta": { "role": "assistant", "content": "working..." },
                                "finish_reason": null
                            }]
                        });
                        Some((
                            Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(
                                format!("data: {chunk}\n\n"),
                            )),
                            1,
                        ))
                    }
                    1 => {
                        // 留出时间窗口以测试 cancel_modern_agent_turn
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        Some((
                            Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(
                                "data: [DONE]\n\n",
                            )),
                            2,
                        ))
                    }
                    _ => None,
                }
            });
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind mock openai server: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("mock listener addr: {e}"))?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{addr}/v1"), handle))
}

#[tokio::test]
async fn nova_serve_stdio_initialize_turn_cancel_and_shutdown() {
    // 固定测试根目录 + 隔离工作区，既隔离 DB/kernel.lock/log，又避免每次随机目录往凭据库新增条目。
    let test_root = std::env::temp_dir().join("nova-cli-serve-lifecycle-it");
    let _ = std::fs::remove_dir_all(&test_root);
    let data_dir = test_root.join("data");
    let workspace_dir = test_root.join("workspace");
    std::fs::create_dir_all(&data_dir).expect("create test data_dir");
    std::fs::create_dir_all(&workspace_dir).expect("create test workspace_dir");

    let (mock_url, mock_task) = start_delayed_mock_openai_server()
        .await
        .expect("start mock server");

    let mut settings = AppSettings {
        provider: "openai".to_string(),
        approval_policy: "never".to_string(),
        ..Default::default()
    };
    settings.provider_profiles.insert(
        "openai".to_string(),
        ProviderProfile {
            display_name: "Mock OpenAI".to_string(),
            api_format: "openai".to_string(),
            api_key: String::new(),
            base_url: mock_url,
            model: "mock-model".to_string(),
            anthropic_thinking_enabled: false,
            anthropic_thinking_budget_tokens: None,
            stop_sequences: Vec::new(),
        },
    );
    settings.normalize_for_runtime();
    let settings_json = serde_json::to_string_pretty(&settings).expect("serialize settings");
    std::fs::write(data_dir.join("settings.json"), settings_json).expect("write settings.json");

    let exe = env!("CARGO_BIN_EXE_nova");
    let mut child = Command::new(exe)
        .arg("serve")
        .env("NOVA_DATA_DIR", &data_dir)
        .env("NOVA_ALLOW_PLAINTEXT_MASTER_KEY", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nova serve");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut events = Vec::new();

    // 1. 错误协议版本应被明确拒绝
    let bad_init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": { "name": "it-test", "version": "0.1" },
            "protocolVersion": "1999-01-01"
        }
    });
    stdin
        .write_all(format!("{bad_init}\n").as_bytes())
        .await
        .expect("write bad init");
    let bad_resp = read_rpc_response(&mut lines, 1, &mut events)
        .await
        .expect("rpc response");
    assert_eq!(bad_resp["error"]["code"].as_i64(), Some(-32600));

    // 2. 合法 initialize 握手
    let init = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": {
            "clientInfo": { "name": "it-test", "version": "0.1" },
            "protocolVersion": nova_protocol::rpc::PROTOCOL_VERSION,
            "capabilities": { "browser": false }
        }
    });
    stdin
        .write_all(format!("{init}\n").as_bytes())
        .await
        .expect("write init");
    let init_resp = read_rpc_response(&mut lines, 2, &mut events)
        .await
        .expect("rpc response");
    assert_eq!(
        init_resp["result"]["protocolVersion"].as_str(),
        Some(nova_protocol::rpc::PROTOCOL_VERSION)
    );

    let initialized = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    stdin
        .write_all(format!("{initialized}\n").as_bytes())
        .await
        .expect("write initialized");

    // 3. 创建会话（带幂等键）并验证幂等重放与结构化 JSON-RPC 错误码
    let conv_resp = rpc_call(
        &mut stdin,
        &mut lines,
        &mut events,
        3,
        "create_conversation",
        json!({
            "title": "Serve Lifecycle",
            "workspacePath": workspace_dir.to_string_lossy(),
            "idempotencyKey": "serve-it-create-conv"
        }),
    )
    .await;
    let conv_id = conv_resp["result"]["id"]
        .as_str()
        .expect("conversation id")
        .to_string();

    let conv_replay = rpc_call(
        &mut stdin,
        &mut lines,
        &mut events,
        7,
        "create_conversation",
        json!({
            "title": "Serve Lifecycle",
            "workspacePath": workspace_dir.to_string_lossy(),
            "idempotencyKey": "serve-it-create-conv"
        }),
    )
    .await;
    assert_eq!(conv_replay["result"]["id"].as_str(), Some(conv_id.as_str()));

    let board_params = json!({
        "conversationId": conv_id,
        "channel": "general",
        "content": "Idempotent hello",
        "idempotencyKey": "serve-it-board-post"
    });
    let board_first = rpc_call(&mut stdin, &mut lines, &mut events, 8, "post_board_message", board_params.clone()).await;
    let board_second = rpc_call(&mut stdin, &mut lines, &mut events, 9, "post_board_message", board_params).await;
    assert_eq!(board_first["result"]["id"], board_second["result"]["id"]);

    let empty_post = rpc_call(
        &mut stdin,
        &mut lines,
        &mut events,
        10,
        "post_board_message",
        json!({
            "conversationId": conv_id,
            "channel": "general",
            "content": "   "
        }),
    )
    .await;
    assert_eq!(empty_post["error"]["code"].as_i64(), Some(-32602));

    let missing_mcp = rpc_call(
        &mut stdin,
        &mut lines,
        &mut events,
        11,
        "get_mcp_server",
        json!({ "name": "missing-server-contract-test" }),
    )
    .await;
    assert_eq!(missing_mcp["error"]["code"].as_i64(), Some(-32002));

    let bad_perm = rpc_call(
        &mut stdin,
        &mut lines,
        &mut events,
        12,
        "submit_permission_decision",
        json!({
            "conversationId": conv_id,
            "requestId": "req-1",
            "action": "invalid_action"
        }),
    )
    .await;
    assert_eq!(bad_perm["error"]["code"].as_i64(), Some(-32602));

    // 4. 发起流式回合，等待首块推送，然后发送 cancel_modern_agent_turn 并校验取消收尾
    let send_turn = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "send_modern_agent_turn",
        "params": {
            "conversationId": conv_id,
            "prompt": "Stream and get cancelled",
            "attachments": null
        }
    });
    stdin
        .write_all(format!("{send_turn}\n").as_bytes())
        .await
        .expect("write send_modern_agent_turn");
    let start_resp = read_rpc_response(&mut lines, 4, &mut events)
        .await
        .expect("rpc response");
    assert_eq!(start_resp["result"]["outcome"].as_str(), Some("started"));

    // 等待收到流式增量或进入运行态后再取消
    let wait_deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let already_saw_delta = events.iter().any(|e| {
            e.get("method").and_then(Value::as_str) == Some("event")
                && e.pointer("/params/name").and_then(Value::as_str) == Some("agent-event")
                && e.pointer("/params/payload/kind").and_then(Value::as_str) == Some("text_delta")
        });
        if already_saw_delta {
            break;
        }
        let rem = wait_deadline.saturating_duration_since(tokio::time::Instant::now());
        if rem.is_zero() {
            break;
        }
        if let Ok(Ok(Some(line))) = tokio::time::timeout(rem, lines.next_line()).await {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                events.push(v);
            }
        }
    }

    let cancel = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "cancel_modern_agent_turn",
        "params": { "conversationId": conv_id }
    });
    stdin
        .write_all(format!("{cancel}\n").as_bytes())
        .await
        .expect("write cancel");
    let _cancel_resp = read_rpc_response(&mut lines, 5, &mut events)
        .await
        .expect("rpc response");

    let cancel_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let cancelled = events.iter().any(|e| {
            e.pointer("/params/name").and_then(Value::as_str) == Some("agent-event")
                && e.pointer("/params/payload/kind").and_then(Value::as_str)
                    == Some("turn_finished")
                && e.pointer("/params/payload/payload/stop_reason").and_then(Value::as_str)
                    == Some("cancelled")
        });
        if cancelled {
            break;
        }
        let rem = cancel_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!rem.is_zero(), "timed out waiting for cancelled turn_finished in events: {events:?}");
        if let Ok(Ok(Some(line))) = tokio::time::timeout(rem, lines.next_line()).await {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                events.push(v);
            }
        }
    }

    // 5. 优雅 shutdown
    let shutdown = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "shutdown",
        "params": {}
    });
    stdin
        .write_all(format!("{shutdown}\n").as_bytes())
        .await
        .expect("write shutdown");
    let shutdown_resp = read_rpc_response(&mut lines, 6, &mut events)
        .await
        .expect("rpc response");
    assert_eq!(shutdown_resp["result"]["ok"].as_bool(), Some(true));

    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("nova serve exits promptly after shutdown")
        .expect("wait child");
    assert!(status.success(), "nova serve exited with status {status}");

    mock_task.abort();
    let _ = std::fs::remove_dir_all(&test_root);
}
