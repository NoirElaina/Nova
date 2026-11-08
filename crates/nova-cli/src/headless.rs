//! 无头一次性执行（`nova -p`）：经内核客户端驱动一个真实回合。
//!
//! 这里不再直连 `nova_core`——与桌面、TUI 一样走 stdio JSON-RPC，
//! 保证内核只有一条接入路径。

use std::io::Write;

use nova_client::Client;
use serde_json::{json, Value};

pub struct HeadlessOptions {
    pub prompt: String,
    pub conversation_id: Option<String>,
    /// 自动放行所有权限请求（等价于逐项 AllowOnce）；缺省一律 DenyOnce（安全默认）。
    pub yes: bool,
}

enum Flow {
    Continue,
    /// 回合终态；`Some` 表示异常结束。
    Finished(Option<String>),
}

pub async fn run(options: HeadlessOptions) -> anyhow::Result<()> {
    let cli_path = std::env::current_exe().map_err(|error| anyhow::anyhow!("{error}"))?;
    let client = Client::spawn(&cli_path).await.map_err(anyhow::Error::msg)?;

    let conversation_id = match options.conversation_id {
        Some(id) => id,
        None => {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let created = client
                .invoke(
                    "create_conversation",
                    json!({ "title": Value::Null, "workspacePath": cwd }),
                )
                .await
                .map_err(anyhow::Error::msg)?;
            created
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("创建会话失败：{}", created))?
                .to_string()
        }
    };

    let mut events = client.subscribe();

    let turn_client = client.clone();
    let turn_conversation = conversation_id.clone();
    let prompt = options.prompt.clone();
    tokio::spawn(async move {
        let _ = turn_client
            .invoke(
                "send_modern_agent_turn",
                json!({
                    "conversationId": turn_conversation,
                    "prompt": prompt,
                    "attachments": Value::Null,
                }),
            )
            .await;
    });

    let mut turn_error: Option<String> = None;
    loop {
        match events.recv().await {
            Ok((name, payload)) if name == "agent-event" => {
                match handle_agent_event(&client, &conversation_id, &payload, options.yes).await {
                    Flow::Continue => {}
                    Flow::Finished(error) => {
                        turn_error = error;
                        break;
                    }
                }
            }
            Ok((name, payload)) if name == "backend-error" || name == "backend-warning" => {
                eprintln!("[{}] {}", name, payload);
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                eprintln!("[warn] 事件流滞后，丢弃 {} 条事件", count);
            }
            Err(_) => break,
        }
    }

    std::io::stdout().flush().ok();
    println!();
    if let Some(error) = turn_error {
        eprintln!("[turn-error] {}", error);
        std::process::exit(1);
    }
    Ok(())
}

async fn handle_agent_event(
    client: &Client,
    conversation_id: &str,
    payload: &Value,
    yes: bool,
) -> Flow {
    let Some(kind) = payload.get("kind").and_then(Value::as_str) else {
        return Flow::Continue;
    };
    let body = payload.get("payload").cloned().unwrap_or(Value::Null);

    match kind {
        "text_delta" => {
            if let Some(delta) = body.get("delta").and_then(Value::as_str) {
                print!("{}", delta);
                std::io::stdout().flush().ok();
            }
            Flow::Continue
        }
        // 无头模式只输出最终回答；思考过程不污染 stdout。
        "thinking_delta" => Flow::Continue,
        "tool_call_requested" => {
            let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("?");
            let args = body.get("arguments").cloned().unwrap_or(Value::Null);
            let summary = args.to_string().chars().take(160).collect::<String>();
            eprintln!("[tool] {} {}", tool, summary);
            Flow::Continue
        }
        "tool_call_completed" => {
            let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("?");
            let is_error = body
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let duration = body.get("duration_ms").and_then(Value::as_u64).unwrap_or(0);
            eprintln!(
                "[tool-done] {} {}{}ms",
                tool,
                if is_error { "ERROR " } else { "" },
                duration
            );
            Flow::Continue
        }
        "permission_requested" => {
            let request_id = body.get("request_id").and_then(Value::as_str).unwrap_or("");
            let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("?");
            let detail = body.get("payload").and_then(Value::as_str).unwrap_or("");
            let action = if yes { "allow_once" } else { "deny_once" };
            eprintln!(
                "[permission] {} {} -> {}",
                tool,
                detail.chars().take(200).collect::<String>(),
                action
            );
            let _ = client
                .invoke(
                    "submit_permission_decision",
                    json!({
                        "conversationId": conversation_id,
                        "requestId": request_id,
                        "action": action,
                    }),
                )
                .await;
            Flow::Continue
        }
        "turn_error" => {
            let error = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            Flow::Finished(Some(error))
        }
        "turn_finished" => Flow::Finished(None),
        _ => Flow::Continue,
    }
}
