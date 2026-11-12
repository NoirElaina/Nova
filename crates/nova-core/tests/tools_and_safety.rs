mod common;

use std::sync::Arc;

use common::{
    openai_text_sse, openai_tool_call_chunked_sse, CapturedLlmRequest, MockLlmResponse,
    MockLlmServer, TestNovaHarness,
};
use nova_core::agent::events::AgentDomainEvent;
use serde_json::{json, Value};

fn extract_last_tool_result_session_id(req: &CapturedLlmRequest) -> Option<i64> {
    let serialized = serde_json::to_string(&req.body).ok()?;
    for marker in ["Process running with session ID ", "\"session_id\":"] {
        if let Some(idx) = serialized.rfind(marker) {
            let rest = serialized[idx + marker.len()..].trim_start();
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(parsed) = digits.parse::<i64>() {
                return Some(parsed);
            }
        }
    }
    None
}

#[tokio::test]
async fn exec_command_write_stdin_interaction_without_background_prompt_injection() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");

    // 第 1 步：启动长驻交互节点进程，输出 READY 并等待 stdin，触发 yield session_id。
    let script = "process.stdout.write('READY_BG\\n'); process.stdin.resume(); process.stdin.on('data', d => { const s = d.toString(); process.stdout.write('ECHO:' + s); if (s.includes('EXIT')) process.exit(0); });";
    let exec_args = serde_json::to_string(&json!({
        "cmd": format!("node -e \"{script}\""),
        "tty": true,
        "yield_time_ms": 1200
    }))
    .expect("serialize exec args");
    let exec_str_boxed: &'static str = Box::leak(exec_args.into_boxed_str());

    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse("call_bg_exec", "exec_command", &[exec_str_boxed]),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue exec_command");

    // 第 2 步：校验不再向 LLM 请求体内重复塞入 `<background_sessions>`，仅通过工具返回 `session_id` 调用 `write_stdin`。
    server
        .push(MockLlmResponse::Dynamic(Arc::new(|req| {
            let body_str = serde_json::to_string(&req.body).unwrap_or_default();
            assert!(
                !body_str.contains("<background_sessions>"),
                "second LLM request must not inject <background_sessions>: {body_str}"
            );
            let session_id = extract_last_tool_result_session_id(req)
                .unwrap_or_else(|| panic!("expected session_id in exec_command tool result: {body_str}"));

            let write_args = serde_json::to_string(&json!({
                "session_id": session_id,
                "chars": "PING_BG\nEXIT\n",
                "yield_time_ms": 2000
            }))
            .unwrap_or_default();
            MockLlmResponse::SseChunks {
                chunks: openai_tool_call_chunked_sse("call_bg_stdin", "write_stdin", &[&write_args]),
                abort_after_chunks: false,
                delay_ms: None,
            }
        })))
        .expect("queue write_stdin dynamic step");

    // 第 3 步：校验 write_stdin 工具结果包含 ECHO:PING_BG，返回最终回答。
    server
        .push(MockLlmResponse::Dynamic(Arc::new(|req| {
            let body_str = serde_json::to_string(&req.body).unwrap_or_default();
            assert!(
                body_str.contains("PING_BG"),
                "write_stdin output should include echoed PING_BG: {body_str}"
            );
            MockLlmResponse::SseChunks {
                chunks: openai_text_sse("interactive background session completed"),
                abort_after_chunks: false,
                delay_ms: None,
            }
        })))
        .expect("queue final step");

    let mut harness = TestNovaHarness::new("openai", "openai", &server.base_url, "never")
        .expect("create harness");
    let conv_id = harness
        .create_conversation("Exec + Stdin + Background Sessions")
        .await
        .expect("create conv");

    let events = harness
        .run_turn(&conv_id, "Start background process and send PING_BG")
        .await
        .expect("turn succeeds");

    assert!(events.iter().any(|e| matches!(
        e,
        AgentDomainEvent::ToolCallCompleted { tool_name, is_error: false, .. } if tool_name == "exec_command"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        AgentDomainEvent::ToolCallCompleted { tool_name, is_error: false, .. } if tool_name == "write_stdin"
    )));

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn apply_patch_rejects_protected_path_bypass_and_traversal() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");

    // 1. 良性首个 hunk + 第二 hunk 写受保护系统路径 (/etc)，测试静态权限防绕过拦截。
    let bypass_patch = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: benign_first.txt\n+ok\n*** Add File: /etc/nova_evil_bypass.conf\n+evil\n*** End Patch"
    }))
    .expect("serialize bypass patch");

    // 2. 路径穿越 (`../escaped_outside.txt`)。
    let traversal_patch = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: ../escaped_outside.txt\n+escape\n*** End Patch"
    }))
    .expect("serialize traversal patch");

    let bypass_leaked: &'static str = Box::leak(bypass_patch.into_boxed_str());
    let traversal_leaked: &'static str = Box::leak(traversal_patch.into_boxed_str());

    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse("call_bypass", "apply_patch", &[bypass_leaked]),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue bypass patch");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse(
                "call_traversal",
                "apply_patch",
                &[traversal_leaked],
            ),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue traversal patch");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_text_sse("both unsafe patches rejected"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue final summary");

    let mut harness = TestNovaHarness::new("openai", "openai", &server.base_url, "on_request")
        .expect("create harness");
    let conv_id = harness
        .create_conversation("ApplyPatch Security Boundary")
        .await
        .expect("create conv");

    let events = harness
        .run_turn_with_permission_decision(&conv_id, "Attempt unsafe patches", "deny_once")
        .await
        .expect("turn finishes cleanly after rejections");

    assert!(
        !harness.workspace_path().join("benign_first.txt").exists(),
        "first hunk must NOT hit disk when second hunk targets /etc"
    );
    if let Some(parent) = harness.workspace_path().parent() {
        assert!(
            !parent.join("escaped_outside.txt").exists(),
            "path traversal target must NOT be written outside workspace"
        );
    }

    let tool_errors = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                AgentDomainEvent::ToolCallCompleted {
                    tool_name,
                    is_error: true,
                    ..
                } if tool_name == "apply_patch"
            )
        })
        .count();
    assert_eq!(tool_errors, 2, "both unsafe apply_patch calls must error");

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn permission_on_request_denies_risky_command_without_executing() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");
    let marker_name = "should_not_exist_when_denied.txt";
    let cmd_args = serde_json::to_string(&json!({
        "cmd": format!("node -e \"require('fs').writeFileSync('{marker_name}', 'pwn')\"")
    }))
    .expect("serialize cmd args");
    let cmd_leaked: &'static str = Box::leak(cmd_args.into_boxed_str());

    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse("call_risky_cmd", "exec_command", &[cmd_leaked]),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue risky exec");
    server
        .push(MockLlmResponse::Dynamic(Arc::new(|req| {
            let body_str = serde_json::to_string(&req.body).unwrap_or_default();
            let has_tool_result = req
                .body
                .get("messages")
                .and_then(Value::as_array)
                .is_some_and(|msgs| {
                    msgs.iter()
                        .any(|m| m.get("role").and_then(Value::as_str) == Some("tool"))
                });
            assert!(has_tool_result, "expected denied tool result in {body_str}");
            MockLlmResponse::SseChunks {
                chunks: openai_text_sse("understood command was denied"),
                abort_after_chunks: false,
                delay_ms: None,
            }
        })))
        .expect("queue post-denial reply");

    let mut harness = TestNovaHarness::new("openai", "openai", &server.base_url, "always_ask")
        .expect("create harness");
    let conv_id = harness
        .create_conversation("Permission Deny Verification")
        .await
        .expect("create conv");

    let events = harness
        .run_turn_with_permission_decision(&conv_id, "Run command to write file", "deny_once")
        .await
        .expect("turn finishes after deny");

    assert!(events
        .iter()
        .any(|e| matches!(e, AgentDomainEvent::PermissionRequested { .. })));
    assert!(
        !harness.workspace_path().join(marker_name).exists(),
        "denied command must never execute"
    );

    harness.shutdown(&[&conv_id]).await;
}
