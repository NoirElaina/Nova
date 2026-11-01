//! 交互式终端 UI：直连内核 Core，强类型事件驱动，无任何 Mock。

use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nova_core::agent::events::AgentDomainEvent;
use nova_core::{Core, NovaEvent};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use tokio::sync::mpsc;

use crate::host;

enum ChatItem {
    User(String),
    Assistant { thinking: String, content: String },
    Tool {
        call_id: String,
        name: String,
        summary: String,
        done: bool,
        is_error: bool,
    },
    Note(String),
}

struct PermissionPrompt {
    request_id: String,
    tool_name: String,
    payload: String,
}

pub struct TuiApp {
    core: Core,
    conversation_id: String,
    events: mpsc::UnboundedReceiver<NovaEvent>,
    messages: Vec<ChatItem>,
    input: String,
    running: bool,
    status: String,
    thinking_open: bool,
    permission: Option<PermissionPrompt>,
    should_exit: bool,
    tokens: (u32, u32),
    model: String,
    scroll: u16,
}

impl TuiApp {
    pub async fn new() -> anyhow::Result<Self> {
        let core = host::build_core()?;
        host::init_kernel(&core).await;

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let conversation = nova_core::api::history::create_conversation(
            &core,
            Some("CLI session".to_string()),
            Some(cwd.clone()),
        )
        .await
        .map_err(anyhow::Error::msg)?;

        let model = nova_core::services::settings::load_settings(&core)
            .map(|s| s.active_provider_profile().model)
            .unwrap_or_else(|_| "unknown".to_string());

        // 内核事件 → TUI 通道（事件泵任务）。
        let (tx, rx) = mpsc::unbounded_channel();
        let mut source = core.subscribe();
        tokio::spawn(async move {
            loop {
                match source.recv().await {
                    Ok(event) => {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(Self {
            core,
            conversation_id: conversation.id,
            events: rx,
            messages: vec![ChatItem::Note(format!(
                "Nova CLI · 工作区 {} · 输入 /help 查看快捷键",
                cwd
            ))],
            input: String::new(),
            running: false,
            status: "idle".to_string(),
            thinking_open: false,
            permission: None,
            should_exit: false,
            tokens: (0, 0),
            model,
            scroll: 0,
        })
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal).await;

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        while !self.should_exit {
            self.drain_events();
            terminal.draw(|frame| self.render(frame))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key).await;
                    }
                }
            }
        }
        Ok(())
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.apply_event(event);
        }
    }

    fn apply_event(&mut self, event: NovaEvent) {
        let NovaEvent::Agent(domain) = event else {
            if let NovaEvent::BackendError(payload) = event {
                self.messages.push(ChatItem::Note(format!(
                    "[backend-error] {}",
                    payload.message
                )));
            } else if let NovaEvent::BackendWarning(payload) = event {
                self.messages.push(ChatItem::Note(format!(
                    "[backend-warning] {}",
                    payload.message
                )));
            }
            return;
        };

        match domain {
            AgentDomainEvent::TextDelta { delta, .. } => {
                self.ensure_assistant();
                if let Some(ChatItem::Assistant { content, .. }) = self.messages.last_mut() {
                    content.push_str(&delta);
                }
            }
            AgentDomainEvent::ThinkingDelta { delta, .. } => {
                self.ensure_assistant();
                if let Some(ChatItem::Assistant { thinking, .. }) = self.messages.last_mut() {
                    thinking.push_str(&delta);
                }
            }
            AgentDomainEvent::ToolCallRequested {
                call_id,
                tool_name,
                arguments,
                ..
            } => {
                let summary = arguments.to_string();
                self.messages.push(ChatItem::Tool {
                    call_id,
                    name: tool_name,
                    summary: summary.chars().take(120).collect(),
                    done: false,
                    is_error: false,
                });
            }
            AgentDomainEvent::ToolCallCompleted {
                call_id,
                is_error,
                duration_ms,
                ..
            } => {
                for item in self.messages.iter_mut().rev() {
                    if let ChatItem::Tool {
                        call_id: id,
                        done,
                        is_error: err,
                        summary,
                        ..
                    } = item
                    {
                        if *id == call_id {
                            *done = true;
                            *err = is_error;
                            summary.push_str(&format!(" ({}ms)", duration_ms));
                            break;
                        }
                    }
                }
            }
            AgentDomainEvent::StateChanged { state, .. } => {
                self.status = format!("{:?}", state).to_lowercase();
            }
            AgentDomainEvent::PermissionRequested {
                request_id,
                tool_name,
                payload,
                ..
            } => {
                self.permission = Some(PermissionPrompt {
                    request_id,
                    tool_name,
                    payload,
                });
            }
            AgentDomainEvent::TokenUsageUpdate {
                input_tokens,
                output_tokens,
                ..
            } => {
                self.tokens = (input_tokens, output_tokens);
            }
            AgentDomainEvent::TurnFinished { .. } => {
                self.running = false;
                self.status = "idle".to_string();
            }
            AgentDomainEvent::TurnError { error, .. } => {
                self.running = false;
                self.status = "idle".to_string();
                self.messages.push(ChatItem::Note(format!("[error] {}", error)));
            }
            AgentDomainEvent::TurnStarted { .. } => {}
        }
    }

    fn ensure_assistant(&mut self) {
        if !matches!(self.messages.last(), Some(ChatItem::Assistant { .. })) {
            self.messages.push(ChatItem::Assistant {
                thinking: String::new(),
                content: String::new(),
            });
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        // 权限弹窗优先截获按键。
        if self.permission.is_some() {
            let action = match key.code {
                KeyCode::Char('y') => Some("allow_once"),
                KeyCode::Char('s') => Some("allow_session"),
                KeyCode::Char('a') => Some("allow_always"),
                KeyCode::Char('n') | KeyCode::Esc => Some("deny_once"),
                _ => None,
            };
            if let Some(action) = action {
                let prompt = self.permission.take().expect("permission checked");
                let result = nova_core::api::permission_rules::submit_permission_decision(
                    &self.core,
                    Some(self.conversation_id.clone()),
                    prompt.request_id,
                    action.to_string(),
                );
                if let Err(error) = result {
                    self.messages
                        .push(ChatItem::Note(format!("[permission-error] {}", error)));
                }
            }
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => {
                    self.should_exit = true;
                    return;
                }
                KeyCode::Char('t') => {
                    self.thinking_open = !self.thinking_open;
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => {
                if self.running {
                    // 真实取消：内核取消令牌驱动回合与工具执行立即中断。
                    nova_core::agent::cancellation::request_cancel(Some(&self.conversation_id));
                    self.status = "cancelling".to_string();
                }
            }
            KeyCode::Enter => {
                let text = self.input.trim().to_string();
                if text.is_empty() {
                    return;
                }
                self.input.clear();
                if text.starts_with('/') {
                    self.handle_command(text).await;
                } else if !self.running {
                    self.submit(text).await;
                } else {
                    self.messages
                        .push(ChatItem::Note("当前回合进行中，Esc 可中断".to_string()));
                }
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(5),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(5),
            _ => {}
        }
    }

    async fn handle_command(&mut self, text: String) {
        match text.as_str() {
            "/exit" => self.should_exit = true,
            "/clear" => {
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                match nova_core::api::history::create_conversation(
                    &self.core,
                    Some("CLI session".to_string()),
                    Some(cwd),
                )
                .await
                {
                    Ok(meta) => {
                        self.conversation_id = meta.id;
                        self.messages.clear();
                        self.messages.push(ChatItem::Note("已开启新会话".to_string()));
                    }
                    Err(error) => self
                        .messages
                        .push(ChatItem::Note(format!("[error] {}", error))),
                }
            }
            "/copy" => {
                let latest = self
                    .messages
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        ChatItem::Assistant { content, .. } if !content.is_empty() => {
                            Some(content.clone())
                        }
                        _ => None,
                    });
                match (latest, arboard::Clipboard::new()) {
                    (Some(text), Ok(mut clipboard)) => {
                        let _ = clipboard.set_text(text);
                        self.messages.push(ChatItem::Note("已复制最近回答".to_string()));
                    }
                    _ => self.messages.push(ChatItem::Note("没有可复制的回答".to_string())),
                }
            }
            "/help" => self.messages.push(ChatItem::Note(
                "Enter 发送 · Esc 中断回合 · Ctrl+T 展开/折叠思考 · PgUp/PgDn 滚动 · /clear 新会话 · /copy 复制回答 · /exit 退出\n权限弹窗：y 允许一次 · s 本会话允许 · a 始终允许 · n 拒绝"
                    .to_string(),
            )),
            other => self
                .messages
                .push(ChatItem::Note(format!("未知命令 {}（/help 查看帮助）", other))),
        }
    }

    async fn submit(&mut self, text: String) {
        self.messages.push(ChatItem::User(text.clone()));
        self.running = true;
        self.status = "assembling".to_string();
        self.scroll = 0;

        let core = self.core.clone();
        let conversation_id = self.conversation_id.clone();
        tokio::spawn(async move {
            if let Err(error) =
                nova_core::agent::send_modern_agent_turn(core, conversation_id, text, None).await
            {
                tracing::error!(%error, "agent turn failed");
            }
        });
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        // ── 消息区 ──
        let mut lines: Vec<Line> = Vec::new();
        for item in &self.messages {
            match item {
                ChatItem::User(text) => {
                    lines.push(Line::from(Span::styled(
                        format!("> {}", text),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::default());
                }
                ChatItem::Assistant { thinking, content } => {
                    if !thinking.is_empty() {
                        if self.thinking_open {
                            for line in thinking.lines() {
                                lines.push(Line::from(Span::styled(
                                    line.to_string(),
                                    Style::default()
                                        .fg(Color::DarkGray)
                                        .add_modifier(Modifier::ITALIC),
                                )));
                            }
                        } else {
                            lines.push(Line::from(Span::styled(
                                format!("〔思考 {} 字符 · Ctrl+T 展开〕", thinking.len()),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                    }
                    if !content.is_empty() {
                        for line in content.lines() {
                            lines.push(Line::from(line.to_string()));
                        }
                    }
                    lines.push(Line::default());
                }
                ChatItem::Tool {
                    name,
                    summary,
                    done,
                    is_error,
                    ..
                } => {
                    let (icon, color) = if !done {
                        ("⚙", Color::Yellow)
                    } else if *is_error {
                        ("✗", Color::Red)
                    } else {
                        ("✓", Color::Green)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("{} {} ", icon, name), Style::default().fg(color)),
                        Span::styled(summary.clone(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
                ChatItem::Note(text) => {
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
            }
        }

        if self.permission.is_some() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "权限审批：y 允许一次 · s 本会话允许 · a 始终允许 · n 拒绝",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        let total = lines.len();
        let viewport = chunks[0].height as usize;
        let bottom = total.saturating_sub(viewport);
        let user_scroll = (self.scroll as usize).min(bottom);
        let messages = Paragraph::new(lines)
            .block(Block::default().borders(Borders::NONE))
            .wrap(Wrap { trim: false })
            .scroll(((bottom - user_scroll) as u16, 0));
        frame.render_widget(messages, chunks[0]);

        // ── 输入区 ──
        let input = Paragraph::new(self.input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" nova · {} ", self.model)),
        );
        frame.render_widget(input, chunks[1]);

        // 光标锚定真实输入位置（IME 候选窗跟随光标，不再漂移）。
        if self.permission.is_none() {
            let cursor_x = chunks[1].x + 1 + self.input.chars().count() as u16;
            let max_x = chunks[1].x + chunks[1].width.saturating_sub(2);
            frame.set_cursor_position((cursor_x.min(max_x), chunks[1].y + 1));
        }

        // ── 状态栏 ──
        let status = if let Some(prompt) = &self.permission {
            format!(
                "⚠ {} 请求权限 · {}",
                prompt.tool_name,
                prompt.payload.chars().take(80).collect::<String>()
            )
        } else {
            format!(
                "{} · tokens ↑{} ↓{} · {}",
                self.status, self.tokens.0, self.tokens.1, self.conversation_id
            )
        };
        let status_line = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(status_line, chunks[2]);
    }
}
