#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use futures_util::stream;
use nova_core::agent::events::AgentDomainEvent;
use nova_core::agent::send_modern_agent_turn;
use nova_core::agent::session::history::close_history_pool;
use nova_core::api::history::{
    append_plain_chat_message, create_conversation, get_conversation_turn_traces, load_history,
};
use nova_core::api::permission_rules::submit_permission_decision;
use nova_core::api::shell::{kill_background_job, list_background_jobs};
use nova_core::services::settings::{invalidate_settings_cache, AppSettings, ProviderProfile};
use nova_core::{Core, CoreConfig, NovaEvent};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct CapturedLlmRequest {
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Value,
}

#[derive(Clone)]
pub enum MockLlmResponse {
    HttpError {
        status: u16,
        body: String,
    },
    SseChunks {
        chunks: Vec<String>,
        abort_after_chunks: bool,
        delay_ms: Option<u64>,
    },
    Json {
        status: u16,
        body: Value,
    },
    Dynamic(Arc<dyn Fn(&CapturedLlmRequest) -> MockLlmResponse + Send + Sync>),
}

struct MockServerState {
    queue: Mutex<VecDeque<MockLlmResponse>>,
    captured: Mutex<Vec<CapturedLlmRequest>>,
    compact_summary: Mutex<Option<String>>,
}

pub struct MockLlmServer {
    pub base_url: String,
    state: Arc<MockServerState>,
    task: JoinHandle<()>,
}

impl Drop for MockLlmServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockLlmServer {
    pub async fn start() -> Result<Self, String> {
        let state = Arc::new(MockServerState {
            queue: Mutex::new(VecDeque::new()),
            captured: Mutex::new(Vec::new()),
            compact_summary: Mutex::new(Some(
                "- Mock compaction summary: earlier conversation compressed.".to_string(),
            )),
        });

        let app = Router::new()
            .route("/v1/messages", post(handle_mock_llm))
            .route("/v1/chat/completions", post(handle_mock_llm))
            .route("/v1/responses", post(handle_mock_llm))
            .with_state(Arc::clone(&state));

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("failed to bind MockLlmServer: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("failed to read MockLlmServer addr: {e}"))?;

        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self {
            base_url: format!("http://{addr}/v1"),
            state,
            task,
        })
    }

    pub fn push(&self, resp: MockLlmResponse) -> Result<(), String> {
        let mut guard = self
            .state
            .queue
            .lock()
            .map_err(|_| "MockLlmServer queue mutex poisoned".to_string())?;
        guard.push_back(resp);
        Ok(())
    }

    pub fn set_compact_summary(&self, summary: Option<String>) -> Result<(), String> {
        let mut guard = self
            .state
            .compact_summary
            .lock()
            .map_err(|_| "MockLlmServer compact_summary mutex poisoned".to_string())?;
        *guard = summary;
        Ok(())
    }

    pub fn captured_requests(&self) -> Result<Vec<CapturedLlmRequest>, String> {
        let guard = self
            .state
            .captured
            .lock()
            .map_err(|_| "MockLlmServer captured mutex poisoned".to_string())?;
        Ok(guard.clone())
    }
}

async fn handle_mock_llm(
    State(state): State<Arc<MockServerState>>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut header_map = HashMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            header_map.insert(name.as_str().to_ascii_lowercase(), v.to_string());
        }
    }

    let path = uri.path().to_string();
    let captured_req = CapturedLlmRequest {
        path: path.clone(),
        headers: header_map,
        body: body.clone(),
    };
    if let Ok(mut captured) = state.captured.lock() {
        captured.push(captured_req.clone());
    }

    // 非流式请求通常来自 compact/summarize 等辅助流程：直接返回匹配协议格式的摘要文本。
    if body.get("stream").and_then(Value::as_bool) == Some(false) {
        if let Ok(summary_guard) = state.compact_summary.lock() {
            if let Some(summary_text) = summary_guard.as_ref() {
                let payload = if path.ends_with("/messages") {
                    json!({
                        "id": "msg_mock_compact",
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "text", "text": summary_text }],
                    })
                } else if path.ends_with("/responses") {
                    json!({
                        "id": "resp_mock_compact",
                        "status": "completed",
                        "output_text": summary_text,
                    })
                } else {
                    json!({
                        "id": "chatcmpl_mock_compact",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": summary_text },
                            "finish_reason": "stop"
                        }]
                    })
                };
                return (StatusCode::OK, axum::Json(payload)).into_response();
            }
        }
    }

    let next_spec = state
        .queue
        .lock()
        .ok()
        .and_then(|mut queue| queue.pop_front());

    let Some(mut spec) = next_spec else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "error": { "message": "MockLlmServer queue is unexpectedly empty" }
            })),
        )
            .into_response();
    };

    while let MockLlmResponse::Dynamic(generator) = spec {
        spec = generator(&captured_req);
    }

    match spec {
        MockLlmResponse::HttpError { status, body } => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Response::builder()
                .status(code)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        MockLlmResponse::Json { status, body } => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
            (code, axum::Json(body)).into_response()
        }
        MockLlmResponse::SseChunks {
            chunks,
            abort_after_chunks,
            delay_ms,
        } => {
            if let Some(ms) = delay_ms {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
            if abort_after_chunks {
                let unfold_stream =
                    stream::unfold((chunks.into_iter(), false), |(mut iter, sent_any)| async move {
                        if let Some(chunk) = iter.next() {
                            return Some((Ok::<Bytes, std::io::Error>(Bytes::from(chunk)), (iter, true)));
                        }
                        if sent_any {
                            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                            return Some((
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::ConnectionReset,
                                    "simulated mid-stream abort",
                                )),
                                (iter, false),
                            ));
                        }
                        None
                    });
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/event-stream")
                    .header("cache-control", "no-cache")
                    .body(Body::from_stream(unfold_stream))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
            let items: Vec<Result<Bytes, std::io::Error>> =
                chunks.into_iter().map(|c| Ok(Bytes::from(c))).collect();
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .body(Body::from_stream(stream::iter(items)))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        MockLlmResponse::Dynamic(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub fn sse_data_frame(event_name: Option<&str>, data: &Value) -> String {
    match event_name {
        Some(name) => format!("event: {name}\ndata: {data}\n\n"),
        None => format!("data: {data}\n\n"),
    }
}

pub fn sse_raw_frame(event_name: Option<&str>, raw_data: &str) -> String {
    match event_name {
        Some(name) => format!("event: {name}\ndata: {raw_data}\n\n"),
        None => format!("data: {raw_data}\n\n"),
    }
}

pub fn anthropic_text_sse(text: &str) -> Vec<String> {
    vec![
        sse_data_frame(
            Some("message_start"),
            &json!({
                "type": "message_start",
                "message": {
                    "content": [],
                    "usage": {
                        "input_tokens": 18,
                        "output_tokens": 1,
                        "cache_read_input_tokens": 0,
                        "cache_creation_input_tokens": 0
                    }
                }
            }),
        ),
        sse_data_frame(
            Some("content_block_start"),
            &json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            }),
        ),
        sse_data_frame(
            Some("content_block_delta"),
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": text }
            }),
        ),
        sse_data_frame(
            Some("content_block_stop"),
            &json!({ "type": "content_block_stop", "index": 0 }),
        ),
        sse_data_frame(
            Some("message_delta"),
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn" },
                "usage": { "output_tokens": 7 }
            }),
        ),
        sse_data_frame(Some("message_stop"), &json!({ "type": "message_stop" })),
    ]
}

pub fn anthropic_tool_use_chunked_sse(
    tool_id: &str,
    tool_name: &str,
    json_fragments: &[&str],
) -> Vec<String> {
    let mut frames = vec![
        sse_data_frame(
            Some("message_start"),
            &json!({
                "type": "message_start",
                "message": {
                    "content": [],
                    "usage": {
                        "input_tokens": 20,
                        "output_tokens": 1,
                        "cache_read_input_tokens": 0,
                        "cache_creation_input_tokens": 0
                    }
                }
            }),
        ),
        sse_data_frame(
            Some("content_block_start"),
            &json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {
                    "type": "tool_use",
                    "id": tool_id,
                    "name": tool_name,
                    "input": {}
                }
            }),
        ),
    ];

    for frag in json_fragments {
        frames.push(sse_data_frame(
            Some("content_block_delta"),
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "input_json_delta", "partial_json": frag }
            }),
        ));
    }

    frames.push(sse_data_frame(
        Some("content_block_stop"),
        &json!({ "type": "content_block_stop", "index": 0 }),
    ));
    frames.push(sse_data_frame(
        Some("message_delta"),
        &json!({
            "type": "message_delta",
            "delta": { "stop_reason": "tool_use" },
            "usage": { "output_tokens": 10 }
        }),
    ));
    frames.push(sse_data_frame(
        Some("message_stop"),
        &json!({ "type": "message_stop" }),
    ));
    frames
}

pub fn openai_text_sse(text: &str) -> Vec<String> {
    vec![
        sse_data_frame(
            None,
            &json!({
                "id": "chatcmpl_1",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": text },
                    "finish_reason": null
                }]
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "id": "chatcmpl_1",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 14, "completion_tokens": 6, "total_tokens": 20 }
            }),
        ),
        sse_raw_frame(None, "[DONE]"),
    ]
}

pub fn openai_tool_call_chunked_sse(
    call_id: &str,
    tool_name: &str,
    json_fragments: &[&str],
) -> Vec<String> {
    let mut frames = vec![sse_data_frame(
        None,
        &json!({
            "id": "chatcmpl_tool_1",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": { "name": tool_name, "arguments": "" }
                    }]
                },
                "finish_reason": null
            }]
        }),
    )];

    for frag in json_fragments {
        frames.push(sse_data_frame(
            None,
            &json!({
                "id": "chatcmpl_tool_1",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "arguments": frag }
                        }]
                    },
                    "finish_reason": null
                }]
            }),
        ));
    }

    frames.push(sse_data_frame(
        None,
        &json!({
            "id": "chatcmpl_tool_1",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 16, "completion_tokens": 9, "total_tokens": 25 }
        }),
    ));
    frames.push(sse_raw_frame(None, "[DONE]"));
    frames
}

pub fn responses_text_sse(text: &str) -> Vec<String> {
    vec![
        sse_data_frame(
            None,
            &json!({
                "type": "response.created",
                "response": { "id": "resp_1", "status": "in_progress" }
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "type": "response.output_text.delta",
                "delta": text
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "usage": { "input_tokens": 12, "output_tokens": 5 }
                }
            }),
        ),
    ]
}

pub fn responses_tool_call_chunked_sse(
    call_id: &str,
    tool_name: &str,
    json_fragments: &[&str],
) -> Vec<String> {
    let mut frames = vec![
        sse_data_frame(
            None,
            &json!({
                "type": "response.created",
                "response": { "id": "resp_tool_1", "status": "in_progress" }
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": "Mock reasoning plan." }]
                }
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": "Mock reasoning plan." }]
                }
            }),
        ),
        sse_data_frame(
            None,
            &json!({
                "type": "response.output_item.added",
                "output_index": 1,
                "item": {
                    "type": "function_call",
                    "call_id": call_id,
                    "name": tool_name,
                    "arguments": ""
                }
            }),
        ),
    ];

    for frag in json_fragments {
        frames.push(sse_data_frame(
            None,
            &json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 1,
                "delta": frag
            }),
        ));
    }

    let full_args: String = json_fragments.concat();
    frames.push(sse_data_frame(
        None,
        &json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "type": "function_call",
                "call_id": call_id,
                "name": tool_name,
                "arguments": full_args
            }
        }),
    ));
    frames.push(sse_data_frame(
        None,
        &json!({
            "type": "response.completed",
            "response": {
                "id": "resp_tool_1",
                "status": "completed",
                "usage": { "input_tokens": 19, "output_tokens": 11 }
            }
        }),
    ));
    frames
}

pub struct TestNovaHarness {
    pub data_dir: TempDir,
    pub workspace_dir: TempDir,
    pub core: Core,
    event_rx: broadcast::Receiver<NovaEvent>,
}

impl TestNovaHarness {
    pub fn new(
        provider_key: &str,
        api_format: &str,
        base_url: &str,
        approval_policy: &str,
    ) -> Result<Self, String> {
        let data_dir = TempDir::new().map_err(|e| format!("failed to create data TempDir: {e}"))?;
        let workspace_dir =
            TempDir::new().map_err(|e| format!("failed to create workspace TempDir: {e}"))?;

        let cfg = CoreConfig::new(data_dir.path().to_path_buf())
            .with_cache_dir(data_dir.path().join("cache"))
            .with_download_dir(data_dir.path().join("downloads"));
        let core = Core::new(cfg, 2048);
        let event_rx = core.subscribe();

        Self::write_settings_file(
            data_dir.path(),
            provider_key,
            api_format,
            base_url,
            approval_policy,
        )?;

        Ok(Self {
            data_dir,
            workspace_dir,
            core,
            event_rx,
        })
    }

    pub fn write_settings_file(
        data_dir: &Path,
        provider_key: &str,
        api_format: &str,
        base_url: &str,
        approval_policy: &str,
    ) -> Result<(), String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| format!("failed to create settings parent dir: {e}"))?;
        let mut settings = AppSettings {
            provider: provider_key.to_string(),
            approval_policy: approval_policy.to_string(),
            progressive_tool_disclosure: false,
            ..Default::default()
        };
        settings.provider_profiles.insert(
            provider_key.to_string(),
            ProviderProfile {
                display_name: format!("Mock {provider_key}"),
                api_format: api_format.to_string(),
                api_key: "sk-mock-secret-test-key-12345".to_string(),
                base_url: base_url.to_string(),
                model: "mock-model".to_string(),
                anthropic_thinking_enabled: false,
                anthropic_thinking_budget_tokens: None,
                stop_sequences: Vec::new(),
            },
        );
        settings.normalize_for_runtime();

        let json_str = serde_json::to_string_pretty(&settings)
            .map_err(|e| format!("failed to serialize mock settings: {e}"))?;
        std::fs::write(data_dir.join("settings.json"), json_str)
            .map_err(|e| format!("failed to write mock settings.json: {e}"))?;
        invalidate_settings_cache();
        Ok(())
    }

    pub fn workspace_path(&self) -> PathBuf {
        self.workspace_dir.path().to_path_buf()
    }

    pub async fn create_conversation(&self, title: &str) -> Result<String, String> {
        let meta = create_conversation(
            &self.core,
            Some(title.to_string()),
            Some(self.workspace_dir.path().to_string_lossy().into_owned()),
            None,
        )
        .await?;
        Ok(meta.id)
    }

    pub async fn seed_history(&self, conv_id: &str, pairs: usize) -> Result<(), String> {
        for idx in 0..pairs {
            append_plain_chat_message(
                &self.core,
                conv_id.to_string(),
                "user".to_string(),
                format!("Earlier seeded question #{idx}"),
            )
            .await?;
            append_plain_chat_message(
                &self.core,
                conv_id.to_string(),
                "assistant".to_string(),
                format!("Earlier seeded answer #{idx}"),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn run_turn(
        &mut self,
        conv_id: &str,
        prompt: &str,
    ) -> Result<Vec<AgentDomainEvent>, String> {
        let (result, events) = self.run_turn_allow_err(conv_id, prompt).await;
        result?;
        Ok(events)
    }

    pub async fn run_turn_allow_err(
        &mut self,
        conv_id: &str,
        prompt: &str,
    ) -> (Result<(), String>, Vec<AgentDomainEvent>) {
        let mut events = Vec::new();
        let turn_result = send_modern_agent_turn(
            self.core.clone(),
            conv_id.to_string(),
            prompt.to_string(),
            None,
        )
        .await;

        while let Ok(evt) = self.event_rx.try_recv() {
            if let NovaEvent::Agent(agent_evt) = evt {
                events.push(agent_evt);
            }
        }

        (turn_result, events)
    }

    /// 针对可能触发 `PermissionRequested` 的回合：并发监听权限弹窗并自动给出裁决。
    pub async fn run_turn_with_permission_decision(
        &mut self,
        conv_id: &str,
        prompt: &str,
        action: &str,
    ) -> Result<Vec<AgentDomainEvent>, String> {
        let core_clone = self.core.clone();
        let conv = conv_id.to_string();
        let prompt_str = prompt.to_string();
        let action_str = action.to_string();

        let turn_handle = tokio::spawn(async move {
            send_modern_agent_turn(core_clone, conv, prompt_str, None).await
        });

        let mut events = Vec::new();
        let mut turn_finished = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);

        while !turn_finished && tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(250), self.event_rx.recv())
                .await
            {
                Ok(Ok(NovaEvent::Agent(agent_evt))) => {
                    if let AgentDomainEvent::PermissionRequested { ref request_id, .. } = agent_evt {
                        let _ = submit_permission_decision(
                            &self.core,
                            Some(conv_id.to_string()),
                            request_id.clone(),
                            action_str.clone(),
                        );
                    }
                    if matches!(agent_evt, AgentDomainEvent::TurnFinished { .. }) {
                        turn_finished = true;
                    }
                    events.push(agent_evt);
                }
                Ok(Ok(_)) => {}
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => {
                    if turn_handle.is_finished() {
                        break;
                    }
                }
            }
        }

        while let Ok(evt) = self.event_rx.try_recv() {
            if let NovaEvent::Agent(agent_evt) = evt {
                events.push(agent_evt);
            }
        }

        let join_res = turn_handle
            .await
            .map_err(|e| format!("turn task panicked or cancelled: {e}"))?;
        join_res?;
        Ok(events)
    }

    pub async fn shutdown(self, conv_ids: &[&str]) {
        for conv_id in conv_ids {
            if let Ok(jobs) = list_background_jobs(Some(conv_id.to_string())).await {
                for job in jobs {
                    let _ = kill_background_job(Some(conv_id.to_string()), job.id).await;
                }
            }
        }
        close_history_pool(&self.core).await;
        drop(self.workspace_dir);
        drop(self.data_dir);
    }

    pub async fn history_count(&self, conv_id: &str) -> Result<usize, String> {
        let history = load_history(&self.core, conv_id.to_string()).await?;
        Ok(history.len())
    }

    pub async fn turn_traces(&self, conv_id: &str) -> Result<Vec<nova_protocol::misc::TurnTrace>, String> {
        get_conversation_turn_traces(&self.core, conv_id.to_string()).await
    }
}
