use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use futures_util::StreamExt;
use nova_client::uds::prepare_private_socket_directory;
use nova_client::{Client, ClientOptions};
use nova_protocol::settings::{AppSettings, ProviderProfile};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::process::Command;

async fn start_approval_mock_openai_server() -> Result<(String, tokio::task::JoinHandle<()>), String> {
    let step = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let step = Arc::clone(&step);
            async move {
                let current = step.fetch_add(1, Ordering::SeqCst);
                let frames: Vec<String> = if current == 0 {
                    let c1 = json!({
                        "id": "cmpl_mw_1",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "role": "assistant",
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_mw_bash_1",
                                    "type": "function",
                                    "function": {
                                        "name": "exec_command",
                                        "arguments": "{\"cmd\":\"echo multi-window\"}"
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    });
                    let c2 = json!({
                        "id": "cmpl_mw_1",
                        "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }]
                    });
                    vec![
                        format!("data: {c1}\n\n"),
                        format!("data: {c2}\n\n"),
                        "data: [DONE]\n\n".to_string(),
                    ]
                } else {
                    let c1 = json!({
                        "id": "cmpl_mw_2",
                        "choices": [{
                            "index": 0,
                            "delta": { "role": "assistant", "content": "denied cleanly" },
                            "finish_reason": null
                        }]
                    });
                    let c2 = json!({
                        "id": "cmpl_mw_2",
                        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
                    });
                    vec![
                        format!("data: {c1}\n\n"),
                        format!("data: {c2}\n\n"),
                        "data: [DONE]\n\n".to_string(),
                    ]
                };

                let stream = futures_util::stream::iter(
                    frames
                        .into_iter()
                        .map(|s| Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(s))),
                )
                .boxed();

                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap_or_else(|_| Response::new(Body::empty()))
                    .into_response()
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{addr}"), handle))
}

#[tokio::test]
async fn shared_uds_daemon_isolates_approvals_and_fails_closed_on_disconnect() {
    let (mock_url, mock_task) = start_approval_mock_openai_server()
        .await
        .expect("start mock openai server");

    let pid = std::process::id();
    let data_root = std::env::temp_dir().join(format!("nova-uds-data-{pid}"));
    let sock_dir = std::env::temp_dir().join(format!("nova-uds-sock-{pid}"));
    let _ = std::fs::remove_dir_all(&data_root);
    let _ = std::fs::remove_dir_all(&sock_dir);
    std::fs::create_dir_all(&data_root).expect("create data_root");
    prepare_private_socket_directory(&sock_dir)
        .await
        .expect("prepare private socket dir");
    let socket_path = sock_dir.join("n.sock");

    let nova_bin = env!("CARGO_BIN_EXE_nova");
    let mut daemon_child = Command::new(nova_bin)
        .arg("serve")
        .arg("--uds")
        .arg(&socket_path)
        .env("NOVA_DATA_DIR", &data_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn nova serve --uds");

    // 等待隔离启动的守护进程完成 UDS 绑定，再测试两扇窗口通过 probe-first `connect_or_spawn` 复用同一进程。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let window_a = loop {
        match Client::connect_uds(&socket_path, ClientOptions::default()).await {
            Ok(client) => break client,
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    let _ = daemon_child.kill().await;
                    panic!("timed out connecting window A to UDS daemon: {err}");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let window_b = Client::connect_or_spawn(nova_bin.as_ref(), &socket_path, ClientOptions::default())
        .await
        .expect("window B reuses running UDS daemon");

    let mut events_a = window_a.subscribe();
    let mut events_b = window_b.subscribe();

    let mut settings = AppSettings {
        provider: "openai".to_string(),
        approval_policy: "always_ask".to_string(),
        ..Default::default()
    };
    settings.provider_profiles.insert(
        "openai".to_string(),
        ProviderProfile {
            display_name: "OpenAI".to_string(),
            api_format: "openai".to_string(),
            api_key: "sk-test-uds-multi-window".to_string(),
            base_url: mock_url,
            model: "gpt-4o-mini".to_string(),
            anthropic_thinking_enabled: false,
            anthropic_thinking_budget_tokens: None,
            stop_sequences: vec![],
        },
    );
    window_a
        .invoke("save_settings", json!({ "settings": settings }))
        .await
        .expect("save settings");

    let created = window_a
        .invoke(
            "create_conversation",
            json!({
                "title": "UDS Multi-Window Approval",
                "workspacePath": data_root.to_string_lossy().to_string(),
                "idempotencyKey": null
            }),
        )
        .await
        .expect("create conversation");
    let conv_id = created["id"]
        .as_str()
        .expect("conversation id")
        .to_string();

    // 确认共享状态：Window B 能立即看见 Window A 创建的会话。
    let convs_b = window_b
        .invoke("list_conversations", json!({}))
        .await
        .expect("list conversations on B");
    assert!(
        convs_b
            .as_array()
            .is_some_and(|arr| arr.iter().any(|c| c.get("id").and_then(Value::as_str) == Some(&conv_id)))
    );

    let submit_res = window_a
        .invoke(
            "send_modern_agent_turn",
            json!({
                "conversationId": conv_id,
                "prompt": "Run echo via Bash",
                "attachments": null,
                "idempotencyKey": null
            }),
        )
        .await
        .expect("send turn from window A");
    assert_eq!(submit_res["outcome"].as_str(), Some("started"));

    // 等待 Window A 收到 `permission_requested`
    let req_id = loop {
        let (name, payload) = tokio::time::timeout(Duration::from_secs(8), events_a.recv())
            .await
            .expect("timeout waiting window A event")
            .expect("window A event stream open");
        if name == "agent-event"
            && payload.get("kind").and_then(Value::as_str) == Some("permission_requested")
        {
            break payload
                .pointer("/payload/request_id")
                .and_then(Value::as_str)
                .expect("request_id")
                .to_string();
        }
    };

    // 校验隔离性：Window B 不得收到该审批弹窗，且手动跨窗口批准必须被拒绝。
    let b_poll_deadline = tokio::time::Instant::now() + Duration::from_millis(200);
    while tokio::time::Instant::now() < b_poll_deadline {
        if let Ok(Some((name, payload))) =
            tokio::time::timeout(Duration::from_millis(40), events_b.recv()).await
        {
            if name == "agent-event"
                && payload.get("kind").and_then(Value::as_str) == Some("permission_requested")
                && payload.pointer("/payload/request_id").and_then(Value::as_str) == Some(&req_id)
            {
                panic!("Window B must NOT receive Window A's PermissionRequested");
            }
        }
    }

    let hijack_err = window_b
        .invoke(
            "submit_permission_decision",
            json!({
                "conversationId": conv_id,
                "requestId": req_id,
                "action": "allow_once"
            }),
        )
        .await
        .expect_err("cross-window approval attempt must fail");
    assert!(
        hijack_err.contains("另一个已连接窗口") || hijack_err.contains("无权"),
        "unexpected error: {hijack_err}"
    );

    // 断开/关闭发起窗口 Window A，待审批必须立即 fail-closed 为 DenyOnce，不得挂死到 15 分钟超时！
    window_a.shutdown();
    drop(events_a);
    drop(window_a);

    let mut saw_denied_tool = false;
    let mut saw_finished = false;
    let finish_deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < finish_deadline {
        let rem = finish_deadline.saturating_duration_since(tokio::time::Instant::now());
        if let Ok(Some((name, payload))) = tokio::time::timeout(rem, events_b.recv()).await {
            if name != "agent-event" {
                continue;
            }
            match payload.get("kind").and_then(Value::as_str) {
                Some("tool_call_completed") => {
                    let is_err = payload.pointer("/payload/is_error").and_then(Value::as_bool);
                    let output = payload
                        .pointer("/payload/output")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if is_err == Some(true) && output.contains("Permission denied by user") {
                        saw_denied_tool = true;
                    }
                }
                Some("turn_finished") => {
                    saw_finished = true;
                    break;
                }
                _ => {}
            }
        }
    }

    assert!(saw_denied_tool, "disconnecting Window A must immediately deny pending tool call fail-closed");
    assert!(saw_finished, "turn must finish cleanly after fail-closed denial");

    let _ = window_b.shutdown_daemon().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_child.wait()).await;
    mock_task.abort();
    let _ = std::fs::remove_dir_all(&data_root);
    let _ = std::fs::remove_dir_all(&sock_dir);
}
