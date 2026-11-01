//! 无头一次性执行（nova -p）：真实内核回合，流式输出，权限请求按安全策略裁决。

use std::io::Write;

use nova_core::agent::events::AgentDomainEvent;
use nova_core::{Core, NovaEvent};

pub struct HeadlessOptions {
    pub prompt: String,
    pub conversation_id: Option<String>,
    /// 自动放行所有权限请求（等价于逐项 AllowOnce）；缺省一律 DenyOnce（安全默认）。
    pub yes: bool,
}

pub async fn run(core: Core, options: HeadlessOptions) -> anyhow::Result<()> {
    let conversation_id = match options.conversation_id {
        Some(id) => id,
        None => {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            nova_core::api::history::create_conversation(
                &core,
                Some("CLI headless".to_string()),
                Some(cwd),
            )
            .await
            .map_err(anyhow::Error::msg)?
            .id
        }
    };

    let mut events = core.subscribe();
    let turn_core = core.clone();
    let turn_conversation = conversation_id.clone();
    let prompt = options.prompt.clone();
    let mut turn = tokio::spawn(async move {
        nova_core::agent::send_modern_agent_turn(turn_core, turn_conversation, prompt, None).await
    });

    let mut turn_error: Option<String> = None;
    loop {
        tokio::select! {
            result = &mut turn => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => turn_error = Some(error),
                    Err(join_error) => turn_error = Some(join_error.to_string()),
                }
                break;
            }
            event = events.recv() => {
                match event {
                    Ok(NovaEvent::Agent(domain)) => {
                        if handle_agent_event(&core, &conversation_id, domain, options.yes).await {
                            // TurnFinished/TurnError 到达后等待 turn 任务收尾即可退出。
                        }
                    }
                    Ok(NovaEvent::BackendError(payload)) => {
                        eprintln!("[backend-error] {}: {}", payload.source, payload.message);
                    }
                    Ok(NovaEvent::BackendWarning(payload)) => {
                        eprintln!("[backend-warning] {}: {}", payload.source, payload.message);
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("[warn] 事件流滞后，丢弃 {} 条事件", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    // 回合结束后排空 stdout。
    std::io::stdout().flush().ok();
    println!();
    if let Some(error) = turn_error {
        eprintln!("[turn-error] {}", error);
        std::process::exit(1);
    }
    Ok(())
}

/// 处理一条 Agent 领域事件；返回是否已到达回合终态。
async fn handle_agent_event(
    core: &Core,
    conversation_id: &str,
    event: AgentDomainEvent,
    yes: bool,
) -> bool {
    match event {
        AgentDomainEvent::TextDelta { delta, .. } => {
            print!("{}", delta);
            std::io::stdout().flush().ok();
        }
        AgentDomainEvent::ThinkingDelta { .. } => {
            // 无头模式只输出最终回答；思考过程不污染 stdout。
        }
        AgentDomainEvent::ToolCallRequested {
            tool_name,
            arguments,
            ..
        } => {
            let summary = arguments.to_string();
            let summary = summary.chars().take(160).collect::<String>();
            eprintln!("[tool] {} {}", tool_name, summary);
        }
        AgentDomainEvent::ToolCallCompleted {
            tool_name,
            is_error,
            duration_ms,
            ..
        } => {
            eprintln!(
                "[tool-done] {} {}{}",
                tool_name,
                if is_error { "ERROR " } else { "" },
                format!("{}ms", duration_ms)
            );
        }
        AgentDomainEvent::PermissionRequested {
            request_id,
            tool_name,
            payload,
            ..
        } => {
            let action = if yes { "allow_once" } else { "deny_once" };
            eprintln!(
                "[permission] {} {} -> {}",
                tool_name,
                payload.chars().take(200).collect::<String>(),
                action
            );
            let _ = nova_core::api::permission_rules::submit_permission_decision(
                core,
                Some(conversation_id.to_string()),
                request_id,
                action.to_string(),
            );
        }
        AgentDomainEvent::TurnError { error, .. } => {
            eprintln!("[error] {}", error);
            return true;
        }
        AgentDomainEvent::TurnFinished { .. } => return true,
        _ => {}
    }
    false
}
