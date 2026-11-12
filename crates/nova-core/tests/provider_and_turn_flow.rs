mod common;

use common::{
    anthropic_text_sse, anthropic_tool_use_chunked_sse, openai_text_sse,
    openai_tool_call_chunked_sse, responses_text_sse, responses_tool_call_chunked_sse,
    sse_data_frame, MockLlmResponse, MockLlmServer, TestNovaHarness,
};
use nova_core::agent::events::AgentDomainEvent;
use serde_json::json;

#[tokio::test]
async fn anthropic_protocol_chunked_tool_call_trace_and_wire_redaction() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");
    let patch_args = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: anthropic_created.txt\n+line 1 from anthropic\n*** End Patch"
    }))
    .expect("serialize patch args");
    let mid = patch_args.len() / 2;
    let frag1 = &patch_args[..mid];
    let frag2 = &patch_args[mid..];

    server
        .push(MockLlmResponse::SseChunks {
            chunks: anthropic_tool_use_chunked_sse("toolu_ant_1", "apply_patch", &[frag1, frag2]),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue tool step");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: anthropic_text_sse("anthropic completed"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue final step");

    let mut harness =
        TestNovaHarness::new("anthropic", "anthropic", &server.base_url, "never")
            .expect("create harness");
    let conv_id = harness
        .create_conversation("Anthropic Integration")
        .await
        .expect("create conv");

    let events = harness
        .run_turn(&conv_id, "Create anthropic_created.txt")
        .await
        .expect("turn succeeds");

    let created = harness.workspace_path().join("anthropic_created.txt");
    let content = std::fs::read_to_string(&created).expect("created file exists");
    assert!(content.contains("line 1 from anthropic"));

    let requests = server.captured_requests().expect("captured");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/v1/messages");
    assert_eq!(requests[1].path, "/v1/messages");

    // 校验 traceId 全链路贯穿与线级报文脱敏
    let turn_finished_trace = events.iter().find_map(|evt| match evt {
        AgentDomainEvent::TurnFinished { trace_id, stop_reason, .. } => {
            assert_eq!(stop_reason, "end_turn");
            trace_id.clone()
        }
        _ => None,
    });
    let trace_id = turn_finished_trace.expect("TurnFinished carries trace_id");
    assert!(!trace_id.is_empty());

    let traces = harness.turn_traces(&conv_id).await.expect("turn traces");
    assert!(traces.iter().any(|t| t.trace_id.as_deref() == Some(trace_id.as_str())));

    let wire_records: Vec<_> = harness
        .core
        .recent_wire_traces()
        .into_iter()
        .filter(|r| r.trace_id.as_deref() == Some(trace_id.as_str()))
        .collect();
    assert!(!wire_records.is_empty(), "expected redacted wire traces");
    for record in &wire_records {
        assert_eq!(record.trace_id.as_deref(), Some(trace_id.as_str()));
        let serialized_payload = record.payload.to_string();
        assert!(
            !serialized_payload.contains("sk-mock-secret-test-key-12345"),
            "secret API key must be redacted from wire buffer: {serialized_payload}"
        );
    }

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn openai_chat_protocol_retries_429_503_and_chunks_tool_calls() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");
    let patch_args = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: openai_created.txt\n+from openai\n*** End Patch"
    }))
    .expect("serialize patch");
    let split = patch_args.len() / 3;
    let p1 = &patch_args[..split];
    let p2 = &patch_args[split..split * 2];
    let p3 = &patch_args[split * 2..];

    server
        .push(MockLlmResponse::HttpError {
            status: 429,
            body: r#"{"error":{"message":"rate limit reached"}}"#.to_string(),
        })
        .expect("queue 429");
    server
        .push(MockLlmResponse::HttpError {
            status: 503,
            body: r#"{"error":{"message":"service temporarily unavailable"}}"#.to_string(),
        })
        .expect("queue 503");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse("call_oai_1", "apply_patch", &[p1, p2, p3]),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue tool chunk stream");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_text_sse("openai completed"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue final text");

    let mut harness = TestNovaHarness::new("openai", "openai", &server.base_url, "never")
        .expect("create harness");
    let conv_id = harness
        .create_conversation("OpenAI Retry + Chunk Tool")
        .await
        .expect("create conv");

    let events = harness
        .run_turn(&conv_id, "Write openai_created.txt")
        .await
        .expect("turn recovers after 429 and 503");

    let content = std::fs::read_to_string(harness.workspace_path().join("openai_created.txt"))
        .expect("file written by openai tool call");
    assert!(content.contains("from openai"));

    let requests = server.captured_requests().expect("captured");
    assert_eq!(requests.len(), 4, "429 + 503 + tool call + final text");
    assert!(requests.iter().all(|r| r.path == "/v1/chat/completions"));
    assert!(events.iter().any(|e| matches!(
        e,
        AgentDomainEvent::ToolCallCompleted { tool_name, is_error: false, .. } if tool_name == "apply_patch"
    )));

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn openai_responses_protocol_reasoning_and_chunked_function_call() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");
    let patch_args = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: responses_created.txt\n+from responses\n*** End Patch"
    }))
    .expect("serialize patch");
    let mid = patch_args.len() / 2;

    server
        .push(MockLlmResponse::SseChunks {
            chunks: responses_tool_call_chunked_sse(
                "fc_resp_1",
                "apply_patch",
                &[&patch_args[..mid], &patch_args[mid..]],
            ),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue responses tool step");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: responses_text_sse("responses completed"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue responses final step");

    let mut harness =
        TestNovaHarness::new("openai", "openai_responses", &server.base_url, "never")
            .expect("create harness");
    let conv_id = harness
        .create_conversation("Responses API Integration")
        .await
        .expect("create conv");

    let events = harness
        .run_turn(&conv_id, "Write responses_created.txt")
        .await
        .expect("responses turn succeeds");

    let content = std::fs::read_to_string(harness.workspace_path().join("responses_created.txt"))
        .expect("file written via responses protocol");
    assert!(content.contains("from responses"));

    let requests = server.captured_requests().expect("captured");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/v1/responses");
    assert_eq!(requests[1].path, "/v1/responses");
    // 第二步应将 function_call_output 传回 Responses API
    let req2_str = serde_json::to_string(&requests[1].body).expect("serialize req2");
    assert!(req2_str.contains("function_call_output"));
    assert!(events.iter().any(|e| matches!(
        e,
        AgentDomainEvent::TextDelta { delta, .. } if delta.contains("responses completed")
    )));

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn mid_stream_abort_after_partial_output_does_not_retry() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");
    let partial_chunk = vec![
        sse_data_frame(
            None,
            &json!({
                "id": "chatcmpl_abort",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": "partial before socket reset" },
                    "finish_reason": null
                }]
            }),
        ),
    ];

    server
        .push(MockLlmResponse::SseChunks {
            chunks: partial_chunk,
            abort_after_chunks: true,
            delay_ms: None,
        })
        .expect("queue aborted partial stream");

    let mut harness = TestNovaHarness::new("openai", "openai", &server.base_url, "never")
        .expect("create harness");
    let conv_id = harness
        .create_conversation("Mid-stream abort test")
        .await
        .expect("create conv");

    let (result, events) = harness
        .run_turn_allow_err(&conv_id, "Trigger mid-stream abort")
        .await;
    assert!(result.is_err(), "aborted mid-stream turn should error out cleanly");

    let requests = server.captured_requests().expect("captured");
    assert_eq!(
        requests.len(),
        1,
        "partial output emitted before stream abort must NEVER retry"
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentDomainEvent::TurnError { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        AgentDomainEvent::TextDelta { delta, .. } if delta.contains("partial before socket reset")
    )));

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn prompt_too_long_triggers_reactive_compaction_and_retries() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");
    server
        .set_compact_summary(Some("Reactive summary checkpoint from mock".to_string()))
        .expect("set compact summary");

    server
        .push(MockLlmResponse::HttpError {
            status: 400,
            body: r#"{"error":{"type":"invalid_request_error","code":"prompt_too_long","message":"prompt_too_long: context window exceeded"}}"#.to_string(),
        })
        .expect("queue prompt_too_long");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_text_sse("recovered after reactive compact"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue compacted retry response");

    let mut harness = TestNovaHarness::new("openai", "openai", &server.base_url, "never")
        .expect("create harness");
    let conv_id = harness
        .create_conversation("Reactive Compact Integration")
        .await
        .expect("create conv");

    // 至少 > REACTIVE_FULL_COMPACT_RECENT_LIMIT (6) + 1 条消息，触发完整模型摘要压缩。
    harness.seed_history(&conv_id, 5).await.expect("seed history");

    let events = harness
        .run_turn(&conv_id, "Final question after long history")
        .await
        .expect("reactive compaction retries and succeeds");

    let requests = server.captured_requests().expect("captured");
    assert_eq!(
        requests.len(),
        3,
        "1: initial too-long streaming request, 2: non-stream compact summary, 3: retried streaming request"
    );
    assert_eq!(
        requests[1].body.get("stream").and_then(|v| v.as_bool()),
        Some(false)
    );
    let retried_body = serde_json::to_string(&requests[2].body).expect("serialize retry body");
    assert!(
        retried_body.contains("Reactive summary checkpoint from mock"),
        "retried request should include compacted summary: {retried_body}"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        AgentDomainEvent::TextDelta { delta, .. } if delta.contains("recovered after reactive compact")
    )));

    harness.shutdown(&[&conv_id]).await;
}
