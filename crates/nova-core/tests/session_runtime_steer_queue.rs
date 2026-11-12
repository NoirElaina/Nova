mod common;

use std::time::Duration;

use common::{
    openai_text_sse, openai_tool_call_chunked_sse, sse_data_frame, MockLlmResponse, MockLlmServer,
    TestNovaHarness,
};
use nova_core::agent::events::AgentDomainEvent;
use nova_core::agent::{prepare_modern_agent_turn, run_prepared_turn_sequence};
use nova_protocol::agent::{TurnSubmitMode, TurnSubmitOutcomeKind};
use serde_json::json;

#[tokio::test]
async fn steer_injects_at_tool_boundary_and_continues_loop() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");

    let patch1 = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: step1.txt\n+from step1\n*** End Patch"
    }))
    .expect("serialize patch1");
    let patch2 = serde_json::to_string(&json!({
        "patch": "*** Begin Patch\n*** Add File: steer.txt\n+from steer\n*** End Patch"
    }))
    .expect("serialize patch2");

    // Step 1：首次工具调用，留出短暂延迟确保 steer 在工具执行期间抵达
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse("call_1", "apply_patch", &[&patch1]),
            abort_after_chunks: false,
            delay_ms: Some(40),
        })
        .expect("queue step 1");
    // Step 2：注入 steer 后继续执行的第二次工具调用
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_tool_call_chunked_sse("call_2", "apply_patch", &[&patch2]),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue step 2");
    // Step 3：终态总结文本
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_text_sse("both files written"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue step 3");

    let harness =
        TestNovaHarness::new("openai", "openai", &server.base_url, "never").expect("harness");
    let conv_id = harness
        .create_conversation("Steer Flow")
        .await
        .expect("create conv");

    let mut rx = harness.core.subscribe();
    let launch = prepare_modern_agent_turn(
        &harness.core,
        &conv_id,
        "Create step1.txt",
        None,
        TurnSubmitMode::StartOrSteer,
        Some("win-a".to_string()),
    )
    .expect("prepare initial turn");
    assert_eq!(launch.outcome.outcome, TurnSubmitOutcomeKind::Started);
    let initial_turn_id = launch.outcome.turn_id.clone();

    let runner_core = harness.core.clone();
    let runner_conv = conv_id.clone();
    let runner = tokio::spawn(async move {
        run_prepared_turn_sequence(runner_core, runner_conv, launch).await
    });

    // 等到第一批工具被请求后立即以 StartOrSteer 注入引导指令
    let steer_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut steered_submitted = false;
    let mut saw_steered_event = false;
    let mut finished = false;

    while tokio::time::Instant::now() < steer_deadline {
        let evt = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("recv event");
        if let Ok(nova_core::NovaEvent::Agent(agent_evt)) = evt {
            match agent_evt {
                AgentDomainEvent::ToolCallRequested { .. } if !steered_submitted => {
                    let steer = prepare_modern_agent_turn(
                        &harness.core,
                        &conv_id,
                        "Also create steer.txt with 'from steer'",
                        None,
                        TurnSubmitMode::StartOrSteer,
                        Some("win-a".to_string()),
                    )
                    .expect("prepare steer");
                    assert_eq!(steer.outcome.outcome, TurnSubmitOutcomeKind::Steered);
                    assert_eq!(steer.outcome.turn_id, initial_turn_id);
                    steered_submitted = true;
                }
                AgentDomainEvent::TurnSteered { prompt, turn_id, .. } => {
                    assert_eq!(turn_id, initial_turn_id);
                    assert!(prompt.contains("steer.txt"));
                    saw_steered_event = true;
                }
                AgentDomainEvent::TurnFinished { stop_reason, .. } => {
                    assert_eq!(stop_reason, "end_turn");
                    finished = true;
                    break;
                }
                _ => {}
            }
        }
    }

    assert!(steered_submitted, "steer should have been submitted mid-turn");
    assert!(saw_steered_event, "TurnSteered event should have been emitted");
    assert!(finished, "turn should finish cleanly");
    runner.await.expect("runner join").expect("turn sequence ok");

    assert!(harness.workspace_path().join("step1.txt").exists());
    assert!(harness.workspace_path().join("steer.txt").exists());

    let captured = server.captured_requests().expect("captured requests");
    assert_eq!(captured.len(), 3);
    assert!(
        captured[1].body.to_string().contains("Also create steer.txt"),
        "second LLM request must contain injected steer prompt, got: {}",
        captured[1].body
    );

    harness.shutdown(&[&conv_id]).await;
}

#[tokio::test]
async fn queue_mode_executes_turns_sequentially_with_distinct_turn_ids() {
    let server = MockLlmServer::start().await.expect("start MockLlmServer");

    // 三个回合依次返回不同正文；第一回合带延迟以确保第 2/3 条严格在运行期排队
    let mut delayed_first = vec![
        sse_data_frame(
            None,
            &json!({
                "id": "cmpl_q1",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": "reply-one" },
                    "finish_reason": null
                }]
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "id": "cmpl_q1",
                "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
            }),
        ),
        "data: [DONE]\n\n".to_string(),
    ];
    server
        .push(MockLlmResponse::SseChunks {
            chunks: std::mem::take(&mut delayed_first),
            abort_after_chunks: false,
            delay_ms: Some(120),
        })
        .expect("queue turn 1");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_text_sse("reply-two"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue turn 2");
    server
        .push(MockLlmResponse::SseChunks {
            chunks: openai_text_sse("reply-three"),
            abort_after_chunks: false,
            delay_ms: None,
        })
        .expect("queue turn 3");

    let harness =
        TestNovaHarness::new("openai", "openai", &server.base_url, "never").expect("harness");
    let conv_id = harness
        .create_conversation("Sequential Queue")
        .await
        .expect("create conv");
    let mut rx = harness.core.subscribe();

    let first = prepare_modern_agent_turn(
        &harness.core,
        &conv_id,
        "first prompt",
        None,
        TurnSubmitMode::Queue,
        Some("win-a".to_string()),
    )
    .expect("first turn");
    assert_eq!(first.outcome.outcome, TurnSubmitOutcomeKind::Started);
    assert_eq!(first.outcome.queue_length, 0);

    let second = prepare_modern_agent_turn(
        &harness.core,
        &conv_id,
        "second prompt",
        None,
        TurnSubmitMode::Queue,
        Some("win-a".to_string()),
    )
    .expect("second turn");
    assert_eq!(second.outcome.outcome, TurnSubmitOutcomeKind::Queued);
    assert_eq!(second.outcome.queue_length, 1);
    assert_ne!(second.outcome.turn_id, first.outcome.turn_id);

    let third = prepare_modern_agent_turn(
        &harness.core,
        &conv_id,
        "third prompt",
        None,
        TurnSubmitMode::Queue,
        Some("win-a".to_string()),
    )
    .expect("third turn");
    assert_eq!(third.outcome.outcome, TurnSubmitOutcomeKind::Queued);
    assert_eq!(third.outcome.queue_length, 2);

    let expected_ids = vec![
        first.outcome.turn_id.clone(),
        second.outcome.turn_id.clone(),
        third.outcome.turn_id.clone(),
    ];

    let core_clone = harness.core.clone();
    let conv_clone = conv_id.clone();
    let runner = tokio::spawn(async move {
        run_prepared_turn_sequence(core_clone, conv_clone, first).await
    });

    runner.await.expect("runner join").expect("all queued turns ok");

    let mut finished_ids = Vec::new();
    while let Ok(evt) = rx.try_recv() {
        if let nova_core::NovaEvent::Agent(AgentDomainEvent::TurnFinished { turn_id, .. }) = evt {
            finished_ids.push(turn_id);
        }
    }
    assert_eq!(finished_ids, expected_ids);

    let captured = server.captured_requests().expect("captured requests");
    assert_eq!(captured.len(), 3);
    assert!(captured[0].body.to_string().contains("first prompt"));
    assert!(captured[1].body.to_string().contains("second prompt"));
    assert!(captured[2].body.to_string().contains("third prompt"));

    harness.shutdown(&[&conv_id]).await;
}
