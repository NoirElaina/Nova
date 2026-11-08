//! 交互式终端 UI：直连内核 Core，全功能终端交互、强类型事件驱动、无任何 Mock。

use std::io;
use std::time::Duration;

use serde_json::{json, Value};
use std::sync::Arc;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nova_core::agent::events::AgentDomainEvent;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Terminal,
};
use tokio::sync::mpsc;


enum ChatItem {
    User(String),
    Assistant {
        thinking: String,
        content: String,
    },
    Tool {
        call_id: String,
        name: String,
        summary: String,
        output: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivePopup {
    SlashCommands,
    ModelSelection,
    ProviderSelection,
    ResumeSessionSelection,
}

#[derive(Debug, Clone)]
struct PopupDisplayItem {
    name: String,
    args: String,
    desc: String,
}

struct SlashCommandDef {
    name: &'static str,
    args: &'static str,
    desc: &'static str,
}

const SLASH_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "/model",
        args: "[name]",
        desc: "查看或切换当前生效 AI 模型",
    },
    SlashCommandDef {
        name: "/provider",
        args: "[name]",
        desc: "查看或切换当前生效模型提供商",
    },
    SlashCommandDef {
        name: "/sessions",
        args: "",
        desc: "列出最近历史会话列表",
    },
    SlashCommandDef {
        name: "/resume",
        args: "<id>",
        desc: "切换并恢复指定历史会话",
    },
    SlashCommandDef {
        name: "/compact",
        args: "",
        desc: "立即压缩当前会话上下文",
    },
    SlashCommandDef {
        name: "/diff",
        args: "",
        desc: "查看工作区代码修改与 Git 差异",
    },
    SlashCommandDef {
        name: "/skills",
        args: "",
        desc: "查看已加载的技能包列表",
    },
    SlashCommandDef {
        name: "/mcp",
        args: "",
        desc: "查看 MCP 服务器运行状态",
    },
    SlashCommandDef {
        name: "/memory",
        args: "",
        desc: "查看跨会话长期记忆库",
    },
    SlashCommandDef {
        name: "/todos",
        args: "",
        desc: "查看当前会话任务待办清单",
    },
    SlashCommandDef {
        name: "/plan",
        args: "",
        desc: "查看当前会话结构化 Plan 规划",
    },
    SlashCommandDef {
        name: "/usage",
        args: "",
        desc: "查看当前会话 Token 与费用统计",
    },
    SlashCommandDef {
        name: "/tools",
        args: "",
        desc: "展开/折叠所有工具执行详情",
    },
    SlashCommandDef {
        name: "/copy",
        args: "",
        desc: "复制最近一条 AI 回答到剪贴板",
    },
    SlashCommandDef {
        name: "/clear",
        args: "",
        desc: "清空屏幕并新建独立会话",
    },
    SlashCommandDef {
        name: "/help",
        args: "",
        desc: "查看键盘快捷键与命令帮助",
    },
    SlashCommandDef {
        name: "/exit",
        args: "",
        desc: "退出 Nova TUI",
    },
];

pub struct TuiApp {
    client: Arc<nova_client::Client>,
    /// 设置缓存：渲染路径是同步的，不能在渲染时就地 await RPC。
    settings: Option<nova_core::services::settings::AppSettings>,
    conversation_id: String,
    events: mpsc::UnboundedReceiver<(String, Value)>,
    messages: Vec<ChatItem>,
    // 输入编辑状态
    input: String,
    cursor_pos: usize, // 字符索引 (0..=input.chars().count())
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    // 运行与显示状态
    running: bool,
    status: String,
    thinking_open: bool,
    tools_expanded: bool, // 工具输出展开状态
    permission: Option<PermissionPrompt>,
    should_exit: bool,
    tokens: (u32, u32),
    model: String,
    scroll: u16,
    // 会话与斜杠命令自动补全选框
    cached_sessions: Vec<nova_protocol::session::ConversationMeta>,
    slash_popup_index: usize,
}

impl TuiApp {
    pub async fn new() -> anyhow::Result<Self> {
        // 经内核客户端接入：与桌面、无头共用一条路径，不再直连 Core。
        let cli_path = std::env::current_exe().map_err(|e| anyhow::anyhow!("{e}"))?;
        let client = nova_client::Client::spawn(&cli_path)
            .await
            .map_err(anyhow::Error::msg)?;

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let conversation = client
            .invoke(
                "create_conversation",
                json!({ "title": Value::Null, "workspacePath": cwd.clone() }),
            )
            .await
            .map_err(anyhow::Error::msg)?;
        let conversation_id = conversation
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("创建会话失败：{}", conversation))?
            .to_string();

        let settings = client
            .invoke("get_settings", json!({}))
            .await
            .ok()
            .and_then(|value| {
                serde_json::from_value::<nova_core::services::settings::AppSettings>(value).ok()
            });
        let model = settings
            .as_ref()
            .map(|s| s.active_provider_profile().model.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let cached_sessions = client
            .invoke("list_conversations", json!({}))
            .await
            .ok()
            .and_then(|value| {
                serde_json::from_value::<Vec<nova_protocol::session::ConversationMeta>>(value).ok()
            })
            .unwrap_or_default();

        // 内核事件 → TUI 通道（事件泵任务）。
        let (tx, rx) = mpsc::unbounded_channel();
        let mut source = client.subscribe();
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
            client,
            settings,
            conversation_id,
            events: rx,
            messages: vec![ChatItem::Note(format!(
                "Nova · 工作区 {} · 输入 / 自动弹出补全，或 /help 查看快捷键",
                cwd
            ))],
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            running: false,
            status: "idle".to_string(),
            thinking_open: false,
            tools_expanded: false,
            permission: None,
            should_exit: false,
            tokens: (0, 0),
            model,
            scroll: 0,
            cached_sessions,
            slash_popup_index: 0,
        })
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal).await;

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
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

            if event::poll(Duration::from_millis(40))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key).await;
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse(mouse);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll = self.scroll.saturating_add(3);
            }
            MouseEventKind::ScrollDown => {
                self.scroll = self.scroll.saturating_sub(3);
            }
            _ => {}
        }
    }

    fn drain_events(&mut self) {
        while let Ok((name, payload)) = self.events.try_recv() {
            self.apply_event(&name, payload);
        }
    }

    /// 统一 RPC 入口：调用内核方法并把 JSON 还原成强类型。
    async fn rpc<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        let value = self.client.invoke(method, params).await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    /// 刷新设置缓存：渲染路径是同步的，不能在渲染时就地 await。
    /// 目前仅在启动时取一次；设置变更后需重启 TUI 才会反映最新值。
    #[allow(dead_code)]
    async fn refresh_settings(&mut self) {
        if let Ok(settings) = self
            .rpc::<nova_core::services::settings::AppSettings>("get_settings", json!({}))
            .await
        {
            self.settings = Some(settings);
        }
    }

    fn apply_event(&mut self, name: &str, payload: Value) {
        match name {
            "agent-event" => {
                // 线协议载荷反序列化回强类型事件，渲染逻辑与直连时期完全一致。
                if let Ok(domain) = serde_json::from_value::<AgentDomainEvent>(payload) {
                    self.apply_agent_event(domain);
                }
            }
            "backend-error" | "backend-warning" => {
                let message = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.messages
                    .push(ChatItem::Note(format!("[{}] {}", name, message)));
            }
            _ => {}
        }
    }

    fn apply_agent_event(&mut self, domain: AgentDomainEvent) {
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
                    output: String::new(),
                    done: false,
                    is_error: false,
                });
            }
            AgentDomainEvent::ToolCallCompleted {
                call_id,
                is_error,
                output,
                duration_ms,
                ..
            } => {
                for item in self.messages.iter_mut().rev() {
                    if let ChatItem::Tool {
                        call_id: id,
                        done,
                        is_error: err,
                        summary,
                        output: out,
                        ..
                    } = item
                    {
                        if *id == call_id {
                            *done = true;
                            *err = is_error;
                            *out = output;
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
                self.messages
                    .push(ChatItem::Note(format!("[error] {}", error)));
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

    // ── 光标与字符串 UTF-8 工具函数 ──

    fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
        s.char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(s.len())
    }

    fn insert_char(&mut self, c: char) {
        let byte_idx = Self::char_to_byte_index(&self.input, self.cursor_pos);
        self.input.insert(byte_idx, c);
        self.cursor_pos += 1;
        self.reset_slash_selection();
    }

    fn insert_str(&mut self, s: &str) {
        let byte_idx = Self::char_to_byte_index(&self.input, self.cursor_pos);
        self.input.insert_str(byte_idx, s);
        self.cursor_pos += s.chars().count();
        self.reset_slash_selection();
    }

    fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            let remove_idx = self.cursor_pos - 1;
            let byte_idx = Self::char_to_byte_index(&self.input, remove_idx);
            self.input.remove(byte_idx);
            self.cursor_pos -= 1;
            self.reset_slash_selection();
        }
    }

    fn delete(&mut self) {
        if self.cursor_pos < self.input.chars().count() {
            let byte_idx = Self::char_to_byte_index(&self.input, self.cursor_pos);
            self.input.remove(byte_idx);
            self.reset_slash_selection();
        }
    }

    fn delete_word_backward(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let chars: Vec<char> = self.input.chars().collect();
        let mut i = self.cursor_pos;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let start_byte = Self::char_to_byte_index(&self.input, i);
        let end_byte = Self::char_to_byte_index(&self.input, self.cursor_pos);
        self.input.replace_range(start_byte..end_byte, "");
        self.cursor_pos = i;
        self.reset_slash_selection();
    }

    fn popup_mode(&self) -> Option<ActivePopup> {
        let trimmed_leading = self.input.trim_start();
        if !trimmed_leading.starts_with('/') {
            return None;
        }

        if !self.input.contains(' ') {
            Some(ActivePopup::SlashCommands)
        } else if self.input.starts_with("/model ") {
            Some(ActivePopup::ModelSelection)
        } else if self.input.starts_with("/provider ") {
            Some(ActivePopup::ProviderSelection)
        } else if self.input.starts_with("/resume ") {
            Some(ActivePopup::ResumeSessionSelection)
        } else {
            None
        }
    }

    fn active_popup_items(&self) -> (&'static str, Vec<PopupDisplayItem>) {
        let Some(mode) = self.popup_mode() else {
            return ("", Vec::new());
        };

        match mode {
            ActivePopup::SlashCommands => {
                let matches: Vec<PopupDisplayItem> = SLASH_COMMANDS
                    .iter()
                    .filter(|cmd| cmd.name.starts_with(&self.input))
                    .map(|cmd| PopupDisplayItem {
                        name: cmd.name.to_string(),
                        args: cmd.args.to_string(),
                        desc: cmd.desc.to_string(),
                    })
                    .collect();
                ("快捷指令", matches)
            }
            ActivePopup::ModelSelection => {
                let query = self.input["/model ".len()..].trim().to_lowercase();
                let mut candidates: Vec<(String, String)> = Vec::new();

                if let Ok(settings) = self.settings.clone().ok_or_else(|| "设置未就绪".to_string()) {
                    let active_provider = settings.active_provider_key();
                    let active_model = settings.active_provider_profile().model;

                    if !active_model.is_empty() {
                        candidates.push((
                            active_model.clone(),
                            format!("✓ 当前生效 · {}", active_provider),
                        ));
                    }

                    if let Some(models) = settings.custom_models.get(&active_provider) {
                        for m in models {
                            candidates.push((
                                m.clone(),
                                format!("自定义模型 · {}", active_provider),
                            ));
                        }
                    }

                    for (p_key, p_prof) in &settings.provider_profiles {
                        if !p_prof.model.is_empty() && p_prof.model != active_model {
                            candidates.push((
                                p_prof.model.clone(),
                                format!("已配置提供商 · {}", p_key),
                            ));
                        }
                    }

                    for (p_key, models) in &settings.custom_models {
                        if p_key != &active_provider {
                            for m in models {
                                candidates.push((
                                    m.clone(),
                                    format!("自定义模型 · {}", p_key),
                                ));
                            }
                        }
                    }
                }

                let standard_models: &[(&str, &str)] = &[
                    ("deepseek-flash", "DeepSeek 极速推理模型"),
                    ("deepseek-chat", "DeepSeek V3 通用对话模型"),
                    ("deepseek-reasoner", "DeepSeek R1 深度思考推理模型"),
                    ("deepseek-coder", "DeepSeek 代码专用模型"),
                    ("gpt-4o", "OpenAI GPT-4o 多模态旗舰"),
                    ("gpt-4o-mini", "OpenAI GPT-4o mini 轻量高速"),
                    ("o1", "OpenAI o1 深度推理"),
                    ("o3-mini", "OpenAI o3-mini 高速推理"),
                    ("claude-3-7-sonnet", "Anthropic Claude 3.7 Sonnet 混合推理"),
                    ("claude-3-5-sonnet", "Anthropic Claude 3.5 Sonnet 编程旗舰"),
                    ("claude-3-5-haiku", "Anthropic Claude 3.5 Haiku 极速模型"),
                    ("gemini-2.5-flash", "Google Gemini 2.5 Flash 1M 超长窗口"),
                    ("gemini-2.5-pro", "Google Gemini 2.5 Pro 2M 极长推理"),
                    ("qwen-2.5-72b-instruct", "通义千问 2.5 72B 旗舰模型"),
                    ("qwen-2.5-coder-32b-instruct", "通义千问 2.5 32B 代码专用"),
                ];

                for &(m, desc) in standard_models {
                    candidates.push((m.to_string(), desc.to_string()));
                }

                let mut seen = std::collections::HashSet::new();
                let mut items = Vec::new();
                for (name, desc) in candidates {
                    let lower = name.to_lowercase();
                    if seen.insert(lower.clone()) {
                        if query.is_empty()
                            || lower.contains(&query)
                            || desc.to_lowercase().contains(&query)
                        {
                            items.push(PopupDisplayItem {
                                name,
                                args: String::new(),
                                desc,
                            });
                        }
                    }
                }
                ("选择 AI 模型", items)
            }
            ActivePopup::ProviderSelection => {
                let query = self.input["/provider ".len()..].trim().to_lowercase();
                let mut candidates: Vec<(String, String)> = Vec::new();

                if let Ok(settings) = self.settings.clone().ok_or_else(|| "设置未就绪".to_string()) {
                    let active = settings.active_provider_key();
                    candidates.push((active.clone(), "✓ 当前生效提供商".to_string()));

                    for (k, prof) in &settings.provider_profiles {
                        if k != &active {
                            candidates.push((
                                k.clone(),
                                format!("已配置 · 绑定模型: {}", prof.model),
                            ));
                        }
                    }
                }

                let standard_providers: &[(&str, &str)] = &[
                    ("deepseek", "DeepSeek 深度求索官方接口"),
                    ("anthropic", "Anthropic Claude 官方接口"),
                    ("openai", "OpenAI 官方接口"),
                    ("google", "Google Gemini 官方接口"),
                    ("ollama", "Ollama 本地大模型服务"),
                    ("openrouter", "OpenRouter 聚合模型路由平台"),
                    ("siliconflow", "SiliconFlow 硅基流动云端推理"),
                ];

                for &(p, desc) in standard_providers {
                    candidates.push((p.to_string(), desc.to_string()));
                }

                let mut seen = std::collections::HashSet::new();
                let mut items = Vec::new();
                for (name, desc) in candidates {
                    let lower = name.to_lowercase();
                    if seen.insert(lower.clone()) {
                        if query.is_empty()
                            || lower.contains(&query)
                            || desc.to_lowercase().contains(&query)
                        {
                            items.push(PopupDisplayItem {
                                name,
                                args: String::new(),
                                desc,
                            });
                        }
                    }
                }
                ("选择模型提供商", items)
            }
            ActivePopup::ResumeSessionSelection => {
                let query = self.input["/resume ".len()..].trim().to_lowercase();
                let mut items = Vec::new();
                for c in &self.cached_sessions {
                    let is_current = c.id == self.conversation_id;
                    let title = if c.title.trim().is_empty() {
                        "新对话"
                    } else {
                        &c.title
                    };
                    let desc = if is_current {
                        format!("✓ [当前] {}", title)
                    } else {
                        title.to_string()
                    };
                    if query.is_empty()
                        || c.id.to_lowercase().contains(&query)
                        || title.to_lowercase().contains(&query)
                    {
                        items.push(PopupDisplayItem {
                            name: c.id.clone(),
                            args: String::new(),
                            desc,
                        });
                    }
                }
                ("恢复历史会话", items)
            }
        }
    }

    fn reset_slash_selection(&mut self) {
        self.slash_popup_index = 0;
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        // 1. 权限审批弹窗优先截获按键
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
                let result = self
                    .client
                    .invoke(
                        "submit_permission_decision",
                        json!({
                            "conversationId": self.conversation_id,
                            "requestId": prompt.request_id,
                            "action": action,
                        }),
                    )
                    .await
                    .map(|_| ());
                if let Err(error) = result {
                    self.messages
                        .push(ChatItem::Note(format!("[permission-error] {}", error)));
                }
            }
            return;
        }

        // 2. 智能补全弹出菜单激活时的按键拦截（一级命令 / 二级模型 / 二级提供商 / 二级会话）
        let popup_mode = self.popup_mode();
        let (_popup_title, popup_items) = self.active_popup_items();
        let has_popup = !popup_items.is_empty();

        if has_popup {
            let selected_idx = self
                .slash_popup_index
                .min(popup_items.len().saturating_sub(1));
            match key.code {
                KeyCode::Up => {
                    if selected_idx > 0 {
                        self.slash_popup_index = selected_idx - 1;
                    } else {
                        self.slash_popup_index = popup_items.len().saturating_sub(1);
                    }
                    return;
                }
                KeyCode::Down => {
                    if selected_idx + 1 < popup_items.len() {
                        self.slash_popup_index = selected_idx + 1;
                    } else {
                        self.slash_popup_index = 0;
                    }
                    return;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    let selected = &popup_items[selected_idx];
                    match popup_mode {
                        Some(ActivePopup::SlashCommands) => {
                            if selected.args.is_empty() && key.code == KeyCode::Enter {
                                // 无参命令直接回车执行
                                let cmd_name = selected.name.clone();
                                self.history.push(cmd_name.clone());
                                self.history_index = None;
                                self.history_draft.clear();
                                self.input.clear();
                                self.cursor_pos = 0;
                                self.reset_slash_selection();
                                self.handle_command(cmd_name).await;
                            } else {
                                // 带有参数的命令（如 /model, /provider, /resume）：补全名称加空格，立即无缝激活二级弹窗！
                                self.input = format!("{} ", selected.name);
                                self.cursor_pos = self.input.chars().count();
                                self.reset_slash_selection();
                            }
                            return;
                        }
                        Some(ActivePopup::ModelSelection) => {
                            // 选中模型！直接执行切换模型
                            let model_name = selected.name.clone();
                            self.history.push(format!("/model {}", model_name));
                            self.history_index = None;
                            self.history_draft.clear();
                            self.input.clear();
                            self.cursor_pos = 0;
                            self.reset_slash_selection();
                            self.handle_command(format!("/model {}", model_name)).await;
                            return;
                        }
                        Some(ActivePopup::ProviderSelection) => {
                            // 选中提供商！直接执行切换提供商
                            let provider_name = selected.name.clone();
                            self.history.push(format!("/provider {}", provider_name));
                            self.history_index = None;
                            self.history_draft.clear();
                            self.input.clear();
                            self.cursor_pos = 0;
                            self.reset_slash_selection();
                            self.handle_command(format!("/provider {}", provider_name)).await;
                            return;
                        }
                        Some(ActivePopup::ResumeSessionSelection) => {
                            // 选中历史会话！直接执行恢复会话
                            let session_id = selected.name.clone();
                            self.history.push(format!("/resume {}", session_id));
                            self.history_index = None;
                            self.history_draft.clear();
                            self.input.clear();
                            self.cursor_pos = 0;
                            self.reset_slash_selection();
                            self.handle_command(format!("/resume {}", session_id)).await;
                            return;
                        }
                        None => {}
                    }
                }
                KeyCode::Esc => {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.reset_slash_selection();
                    return;
                }
                _ => {}
            }
        }

        // 3. 全局 Ctrl 快捷键处理
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
                KeyCode::Char('o') => {
                    self.tools_expanded = !self.tools_expanded;
                    return;
                }
                KeyCode::Char('a') => {
                    self.cursor_pos = 0;
                    return;
                }
                KeyCode::Char('e') => {
                    self.cursor_pos = self.input.chars().count();
                    return;
                }
                KeyCode::Char('u') => {
                    let byte_idx = Self::char_to_byte_index(&self.input, self.cursor_pos);
                    self.input.drain(..byte_idx);
                    self.cursor_pos = 0;
                    self.reset_slash_selection();
                    return;
                }
                KeyCode::Char('k') => {
                    let byte_idx = Self::char_to_byte_index(&self.input, self.cursor_pos);
                    self.input.truncate(byte_idx);
                    self.reset_slash_selection();
                    return;
                }
                KeyCode::Char('w') => {
                    self.delete_word_backward();
                    return;
                }
                KeyCode::Char('v') => {
                    // 剪贴板快速粘贴
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if let Ok(text) = clipboard.get_text() {
                            self.insert_str(&text);
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        // 4. 常规键盘编辑处理
        match key.code {
            KeyCode::Esc => {
                if self.running {
                    nova_core::agent::cancellation::request_cancel(Some(&self.conversation_id));
                    self.status = "cancelling".to_string();
                }
            }
            KeyCode::Enter => {
                // 如果按下了 Alt 或 Shift，或者行末为反斜杠，则换行输入
                if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    self.insert_char('\n');
                    return;
                }
                let text = self.input.trim().to_string();
                if text.is_empty() {
                    return;
                }
                // 存入历史记录
                self.history.push(self.input.clone());
                self.history_index = None;
                self.history_draft.clear();

                self.input.clear();
                self.cursor_pos = 0;

                if text.starts_with('/') {
                    self.handle_command(text).await;
                } else if !self.running {
                    self.submit(text).await;
                } else {
                    self.messages
                        .push(ChatItem::Note("当前回合进行中，Esc 可中断".to_string()));
                }
            }
            KeyCode::Left => {
                self.cursor_pos = self.cursor_pos.saturating_sub(1);
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input.chars().count() {
                    self.cursor_pos += 1;
                }
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
            }
            KeyCode::End => {
                self.cursor_pos = self.input.chars().count();
            }
            KeyCode::Up => {
                // 命令历史回溯（上一条）
                if !self.history.is_empty() {
                    match self.history_index {
                        None => {
                            self.history_draft = self.input.clone();
                            let idx = self.history.len() - 1;
                            self.history_index = Some(idx);
                            self.input = self.history[idx].clone();
                            self.cursor_pos = self.input.chars().count();
                        }
                        Some(idx) if idx > 0 => {
                            let next_idx = idx - 1;
                            self.history_index = Some(next_idx);
                            self.input = self.history[next_idx].clone();
                            self.cursor_pos = self.input.chars().count();
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Down => {
                // 命令历史回溯（下一条）
                match self.history_index {
                    Some(idx) if idx + 1 < self.history.len() => {
                        let next_idx = idx + 1;
                        self.history_index = Some(next_idx);
                        self.input = self.history[next_idx].clone();
                        self.cursor_pos = self.input.chars().count();
                    }
                    Some(_) => {
                        self.history_index = None;
                        self.input = self.history_draft.clone();
                        self.cursor_pos = self.input.chars().count();
                    }
                    None => {}
                }
            }
            KeyCode::Backspace => {
                self.backspace();
            }
            KeyCode::Delete => {
                self.delete();
            }
            KeyCode::Char(c) => {
                self.insert_char(c);
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(5),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(5),
            _ => {}
        }
    }

    async fn handle_command(&mut self, text: String) {
        let trimmed = text.trim();
        let (cmd, args) = match trimmed.split_once(' ') {
            Some((c, a)) => (c, a.trim()),
            None => (trimmed, ""),
        };

        match cmd {
            "/exit" => self.should_exit = true,
            "/clear" => {
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                match self
                    .rpc::<nova_protocol::session::ConversationMeta>(
                        "create_conversation",
                        json!({ "title": Value::Null, "workspacePath": cwd }),
                    )
                    .await
                {
                    Ok(meta) => {
                        self.conversation_id = meta.id;
                        self.messages.clear();
                        self.messages.push(ChatItem::Note("已开启新会话".to_string()));
                        if let Ok(convs) = self
                            .rpc::<Vec<nova_protocol::session::ConversationMeta>>(
                                "list_conversations",
                                json!({}),
                            )
                            .await
                        {
                            self.cached_sessions = convs;
                        }
                    }
                    Err(error) => self
                        .messages
                        .push(ChatItem::Note(format!("[error] {}", error))),
                }
            }
            "/tools" => {
                self.tools_expanded = !self.tools_expanded;
                let status_msg = if self.tools_expanded {
                    "已展开所有工具输出详情 (Ctrl+O 折叠)"
                } else {
                    "已折叠工具输出详情 (Ctrl+O 展开)"
                };
                self.messages.push(ChatItem::Note(status_msg.to_string()));
            }
            "/copy" => {
                let latest = self.messages.iter().rev().find_map(|item| match item {
                    ChatItem::Assistant { content, .. } if !content.is_empty() => {
                        Some(content.clone())
                    }
                    _ => None,
                });
                match (latest, arboard::Clipboard::new()) {
                    (Some(text), Ok(mut clipboard)) => {
                        let _ = clipboard.set_text(text);
                        self.messages
                            .push(ChatItem::Note("已成功复制最近一条回答".to_string()));
                    }
                    _ => self
                        .messages
                        .push(ChatItem::Note("没有可复制的回答".to_string())),
                }
            }
            "/compact" => {
                self.messages
                    .push(ChatItem::Note("正在压缩当前对话上下文...".to_string()));
                match self
                    .rpc::<nova_core::agent::capabilities::compact::ManualCompactOutcome>(
                        "manual_compact_conversation",
                        json!({ "conversationId": self.conversation_id }),
                    )
                    .await
                {
                    Ok(outcome) => {
                        self.messages.push(ChatItem::Note(format!(
                            "上下文压缩完成：节省约 {} tokens ({} -> {})",
                            outcome.saved_tokens, outcome.before_tokens, outcome.after_tokens
                        )));
                    }
                    Err(err) => {
                        self.messages
                            .push(ChatItem::Note(format!("[compact-error] 压缩失败: {}", err)));
                    }
                }
            }
            "/model" => {
                if args.is_empty() {
                    match self.settings.clone().ok_or_else(|| "设置未就绪".to_string()) {
                        Ok(settings) => {
                            let profile = settings.active_provider_profile();
                            let mut lines = vec![
                                format!("当前生效提供商: {}", settings.provider),
                                format!("当前模型: {}", profile.model),
                                format!("API 格式: {}", profile.api_format),
                            ];
                            if let Some(models) = settings.custom_models.get(&settings.provider) {
                                if !models.is_empty() {
                                    lines.push(format!("该提供商配置的模型列表: {}", models.join(", ")));
                                }
                            }
                            lines.push("\n提示：使用 /model <模型名> 可快速切换".to_string());
                            self.messages.push(ChatItem::Note(lines.join("\n")));
                        }
                        Err(e) => {
                            self.messages
                                .push(ChatItem::Note(format!("[model-error] {}", e)));
                        }
                    }
                } else {
                    match self.settings.clone().ok_or_else(|| "设置未就绪".to_string()) {
                        Ok(mut settings) => {
                            let key = settings.active_provider_key();
                            settings.provider_profiles.entry(key).or_default().model = args.to_string();
                            if let Err(e) = self
                                .rpc::<Value>("save_settings", json!({ "settings": settings }))
                                .await
                            {
                                self.messages
                                    .push(ChatItem::Note(format!("[model-save-error] {}", e)));
                            } else {
                                self.model = args.to_string();
                                self.messages
                                    .push(ChatItem::Note(format!("模型已成功切换为: {}", args)));
                            }
                        }
                        Err(e) => {
                            self.messages
                                .push(ChatItem::Note(format!("[model-error] {}", e)));
                        }
                    }
                }
            }
            "/provider" => {
                if args.is_empty() {
                    match self.settings.clone().ok_or_else(|| "设置未就绪".to_string()) {
                        Ok(settings) => {
                            let mut providers: Vec<String> =
                                settings.provider_profiles.keys().cloned().collect();
                            providers.sort();
                            let list = if providers.is_empty() {
                                "无".to_string()
                            } else {
                                providers.join(", ")
                            };
                            self.messages.push(ChatItem::Note(format!(
                                "当前生效提供商: {}\n已配置提供商: {}\n提示：使用 /provider <提供商名> 可快速切换",
                                settings.provider, list
                            )));
                        }
                        Err(e) => {
                            self.messages
                                .push(ChatItem::Note(format!("[provider-error] {}", e)));
                        }
                    }
                } else {
                    match self.settings.clone().ok_or_else(|| "设置未就绪".to_string()) {
                        Ok(mut settings) => {
                            let old_provider = settings.provider.clone();
                            settings.provider = args.to_string();
                            let new_model = settings.active_provider_profile().model;
                            if let Err(e) = self
                                .rpc::<Value>("save_settings", json!({ "settings": settings }))
                                .await
                            {
                                self.messages
                                    .push(ChatItem::Note(format!("[provider-save-error] {}", e)));
                            } else {
                                self.model = new_model.clone();
                                self.messages.push(ChatItem::Note(format!(
                                    "提供商已从 {} 切换为 {} (当前生效模型: {})",
                                    old_provider, args, new_model
                                )));
                            }
                        }
                        Err(e) => {
                            self.messages
                                .push(ChatItem::Note(format!("[provider-error] {}", e)));
                        }
                    }
                }
            }
            "/sessions" => {
                match self
                    .rpc::<Vec<nova_protocol::session::ConversationMeta>>(
                        "list_conversations",
                        json!({}),
                    )
                    .await
                {
                    Ok(conversations) => {
                        self.cached_sessions = conversations.clone();
                        let mut lines = vec![
                            format!("历史会话列表 (共 {} 个，使用 /resume <id> 恢复):", conversations.len()),
                        ];
                        for c in conversations.iter().take(15) {
                            let is_curr = if c.id == self.conversation_id {
                                "[当前] "
                            } else {
                                "       "
                            };
                            let title = if c.title.trim().is_empty() {
                                "无标题"
                            } else {
                                &c.title
                            };
                            lines.push(format!("{}· {} ({})", is_curr, c.id, title));
                        }
                        self.messages.push(ChatItem::Note(lines.join("\n")));
                    }
                    Err(err) => {
                        self.messages
                            .push(ChatItem::Note(format!("[sessions-error] {}", err)));
                    }
                }
            }
            "/resume" => {
                if args.is_empty() {
                    self.messages
                        .push(ChatItem::Note("用法: /resume <会话ID>（输入 /sessions 可查看列表）".to_string()));
                    return;
                }
                match self
                    .rpc::<Vec<nova_protocol::session::HistoryMessage>>(
                        "load_history",
                        json!({ "conversationId": args }),
                    )
                    .await
                {
                    Ok(history) => {
                        self.conversation_id = args.to_string();
                        self.messages.clear();
                        self.messages
                            .push(ChatItem::Note(format!("已成功恢复会话: {}", args)));
                        for msg in history {
                            if msg.role == "user" {
                                self.messages.push(ChatItem::User(msg.content));
                            } else if msg.role == "assistant" {
                                self.messages.push(ChatItem::Assistant {
                                    thinking: msg.reasoning.unwrap_or_default(),
                                    content: msg.content,
                                });
                            }
                        }
                    }
                    Err(err) => {
                        self.messages.push(ChatItem::Note(format!(
                            "[resume-error] 找不到或无法读取会话 {}: {}",
                            args, err
                        )));
                    }
                }
            }
            "/skills" => {
                match self
                    .rpc::<Vec<nova_protocol::misc::SkillSummary>>("list_skills", json!({}))
                    .await
                {
                    Ok(skills) => {
                        let mut lines = vec![format!("已加载技能包列表 (共 {} 个):", skills.len())];
                        for s in skills.iter().take(20) {
                            let desc_preview: String = s.description.chars().take(50).collect();
                            lines.push(format!("• {:<16} - {}", s.name, desc_preview));
                        }
                        self.messages.push(ChatItem::Note(lines.join("\n")));
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[skills-error] {}", e)));
                    }
                }
            }
            "/mcp" => {
                match self
                    .rpc::<Vec<nova_protocol::mcp::McpServerStatus>>(
                        "get_mcp_server_statuses",
                        json!({}),
                    )
                    .await
                {
                    Ok(statuses) => {
                        let mut lines = vec![format!("MCP 服务器运行状态 (共 {} 个):", statuses.len())];
                        for st in statuses {
                            let icon = if st.status == "connected" { "✓" } else { "✗" };
                            lines.push(format!(
                                "{} {:<16} [{}] 工具数: {}",
                                icon, st.name, st.status, st.tool_count
                            ));
                        }
                        self.messages.push(ChatItem::Note(lines.join("\n")));
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[mcp-error] {}", e)));
                    }
                }
            }
            "/memory" => {
                match self
                    .rpc::<Vec<String>>("list_memory_entries", json!({}))
                    .await
                {
                    Ok(entries) => {
                        let mut lines = vec![format!("跨会话持久记忆 (共 {} 条):", entries.len())];
                        for (i, entry) in entries.iter().enumerate() {
                            lines.push(format!("{}. {}", i + 1, entry));
                        }
                        self.messages.push(ChatItem::Note(lines.join("\n")));
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[memory-error] {}", e)));
                    }
                }
            }
            "/diff" => {
                let cid = self.conversation_id.clone();
                match self
                    .rpc::<nova_protocol::misc::WorkspaceDiff>(
                        "get_workspace_diff",
                        json!({ "conversationId": cid }),
                    )
                    .await
                {
                    Ok(diff) => {
                        let mut lines = vec![format!(
                            "工作区代码变更 (共 {} 个文件，+{} -{}):",
                            diff.files.len(),
                            diff.total_additions,
                            diff.total_deletions
                        )];
                        for f in diff.files.iter().take(20) {
                            lines.push(format!(
                                "• [{}] {:<24} (+{} -{})",
                                f.change_type, f.path, f.additions, f.deletions
                            ));
                        }
                        self.messages.push(ChatItem::Note(lines.join("\n")));
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[diff-error] {}", e)));
                    }
                }
            }
            "/todos" => {
                match self
                    .rpc::<Vec<nova_protocol::misc::TodoEntry>>(
                        "list_todos",
                        json!({ "conversationId": self.conversation_id }),
                    )
                    .await
                {
                    Ok(todos) => {
                        if todos.is_empty() {
                            self.messages
                                .push(ChatItem::Note("当前会话暂无待办事项 (Todos)。".to_string()));
                        } else {
                            let mut lines = vec![format!("当前待办事项清单 (共 {} 项):", todos.len())];
                            for item in todos {
                                let status_icon = match item.status.as_str() {
                                    "completed" => "[✓]",
                                    "in_progress" => "[▶]",
                                    _ => "[ ]",
                                };
                                lines.push(format!(
                                    "{} #{} {} ({}, 优先级: {})",
                                    status_icon, item.id, item.content, item.status, item.priority
                                ));
                            }
                            self.messages.push(ChatItem::Note(lines.join("\n")));
                        }
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[todos-error] {}", e)));
                    }
                }
            }
            "/plan" => {
                match self
                    .rpc::<Option<nova_protocol::misc::ConversationPlan>>(
                        "get_conversation_plan",
                        json!({ "conversationId": self.conversation_id }),
                    )
                    .await
                {
                    Ok(Some(plan)) => {
                        let mut lines = vec![format!(
                            "当前会话任务规划 (Plan · 更新时间戳: {}):",
                            plan.updated_at
                        )];
                        lines.push(plan.content);
                        self.messages.push(ChatItem::Note(lines.join("\n")));
                    }
                    Ok(None) => {
                        self.messages
                            .push(ChatItem::Note("当前会话暂无结构化 Plan 规划。".to_string()));
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[plan-error] {}", e)));
                    }
                }
            }
            "/usage" => {
                match self
                    .rpc::<nova_protocol::misc::ConversationUsageSummary>(
                        "get_conversation_usage",
                        json!({ "conversationId": self.conversation_id }),
                    )
                    .await
                {
                    Ok(usage) => {
                        let summary = format!(
                            "当前会话 Token 消耗统计:\n  \
                             调用请求: {} 次\n  \
                             输入 Tokens: {}\n  \
                             输出 Tokens: {}\n  \
                             缓存读取: {}\n  \
                             缓存写入: {}\n  \
                             总计 Tokens: {}\n  \
                             预估费用: ${} USD",
                            usage.requests,
                            usage.input_tokens,
                            usage.output_tokens,
                            usage.cache_read_tokens,
                            usage.cache_creation_tokens,
                            usage.total_tokens,
                            usage.total_cost_usd
                        );
                        self.messages.push(ChatItem::Note(summary));
                    }
                    Err(e) => {
                        self.messages
                            .push(ChatItem::Note(format!("[usage-error] {}", e)));
                    }
                }
            }
            "/help" => self.messages.push(ChatItem::Note(
                "快捷键指南：\n  \
                 Enter 发送 · Alt/Shift+Enter 换行 · Esc 中断当前回合\n  \
                 ←/→ 移动光标 · Home/End 移至行首行尾 · Ctrl+W 删词 · Ctrl+V 粘贴\n  \
                 ↑/↓ 调出输入历史 · 鼠标滚轮/PgUp/PgDn 上下滚动消息\n  \
                 Ctrl+T 展开/折叠思考链 · Ctrl+O 展开/折叠工具执行日志\n\n\
                 常用斜杠命令 (输入 / 自动弹出补全，↑↓ 切换，Tab/Enter 补全)：\n  \
                 /model [name]    查看或切换当前生效 AI 模型\n  \
                 /provider [name] 查看或切换当前模型提供商\n  \
                 /sessions        列出最近历史会话列表\n  \
                 /resume <id>     切换并恢复指定会话\n  \
                 /compact         立即手动压缩当前上下文\n  \
                 /diff            查看工作区代码修改与 Git 差异\n  \
                 /skills          查看已安装与加载的技能包\n  \
                 /mcp             查看 MCP 服务运行状态与工具数\n  \
                 /memory          查看跨会话长期全局记忆\n  \
                 /todos           查看当前会话任务待办清单\n  \
                 /plan            查看当前会话结构化 Plan 规划\n  \
                 /usage           查看当前会话 Token 消耗与费用\n  \
                 /tools           展开/折叠所有工具详细输出\n  \
                 /copy            复制最近一条 AI 回答到剪贴板\n  \
                 /clear           清空屏幕并新建独立会话\n  \
                 /exit            退出 Nova TUI\n\n\
                 权限审批弹窗：y 允许一次 · s 本会话允许 · a 始终允许 · n 拒绝"
                    .to_string(),
            )),
            other => self
                .messages
                .push(ChatItem::Note(format!("未知命令 {}（/help 查看完整帮助）", other))),
        }
    }

    async fn submit(&mut self, text: String) {
        self.messages.push(ChatItem::User(text.clone()));
        self.running = true;
        self.status = "assembling".to_string();
        self.scroll = 0;

        let client = self.client.clone();
        let conversation_id = self.conversation_id.clone();
        tokio::spawn(async move {
            if let Err(error) = client
                .invoke(
                    "send_modern_agent_turn",
                    json!({
                        "conversationId": conversation_id,
                        "prompt": text,
                        "attachments": Value::Null,
                    }),
                )
                .await
            {
                tracing::error!(%error, "agent turn failed");
            }
        });
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        // 动态输入框高度：根据输入行数自适应（单行 3 高度，多行最高 7）
        let line_count = self.input.split('\n').count().clamp(1, 5) as u16;
        let input_height = line_count + 2;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(input_height),
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
                                    format!("💭 {}", line),
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
                        let mut in_code_block = false;
                        for line in content.lines() {
                            if line.starts_with("```") {
                                in_code_block = !in_code_block;
                                lines.push(Line::from(Span::styled(
                                    line.to_string(),
                                    Style::default().fg(Color::Yellow),
                                )));
                            } else if in_code_block {
                                lines.push(Line::from(vec![
                                    Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                                    Span::styled(line.to_string(), Style::default().fg(Color::Green)),
                                ]));
                            } else if line.starts_with("# ") {
                                lines.push(Line::from(Span::styled(
                                    line.to_string(),
                                    Style::default()
                                        .fg(Color::Yellow)
                                        .add_modifier(Modifier::BOLD),
                                )));
                            } else if line.starts_with("## ") || line.starts_with("### ") {
                                lines.push(Line::from(Span::styled(
                                    line.to_string(),
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .add_modifier(Modifier::BOLD),
                                )));
                            } else if line.starts_with("> ") {
                                lines.push(Line::from(Span::styled(
                                    line.to_string(),
                                    Style::default()
                                        .fg(Color::DarkGray)
                                        .add_modifier(Modifier::ITALIC),
                                )));
                            } else {
                                lines.push(Line::from(line.to_string()));
                            }
                        }
                    }
                    lines.push(Line::default());
                }
                ChatItem::Tool {
                    name,
                    summary,
                    output,
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

                    // 如果开启了工具详情展开，或者发生了错误，渲染工具输出
                    if (self.tools_expanded || *is_error) && !output.trim().is_empty() {
                        let out_lines: Vec<&str> = output.lines().collect();
                        let max_display = if self.tools_expanded { 30 } else { 8 };
                        let display_count = out_lines.len().min(max_display);
                        for line in &out_lines[..display_count] {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "   │ ",
                                    Style::default().fg(if *is_error {
                                        Color::Red
                                    } else {
                                        Color::DarkGray
                                    }),
                                ),
                                Span::styled(
                                    line.to_string(),
                                    Style::default().fg(if *is_error {
                                        Color::LightRed
                                    } else {
                                        Color::Gray
                                    }),
                                ),
                            ]));
                        }
                        if out_lines.len() > max_display {
                            lines.push(Line::from(Span::styled(
                                format!("   │ ... 还有 {} 行 (输入 /tools 展开全部)", out_lines.len() - max_display),
                                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                            )));
                        }
                    }
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
        let inner_width = chunks[1].width.saturating_sub(2) as usize;
        let input_widget = if line_count == 1 {
            let chars: Vec<char> = self.input.chars().collect();
            let display_start = if self.cursor_pos >= inner_width {
                self.cursor_pos - inner_width + 1
            } else {
                0
            };
            let visible_input: String =
                chars.iter().skip(display_start).take(inner_width).collect();
            Paragraph::new(visible_input)
        } else {
            Paragraph::new(self.input.as_str()).wrap(Wrap { trim: false })
        };

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if self.running {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Cyan)
            })
            .title(format!(
                " nova · {} (Enter 发送 / Alt+Enter 换行 / Esc 中断) ",
                self.model
            ));
        frame.render_widget(input_widget.block(input_block), chunks[1]);

        // 光标锚定真实输入位置（支持多行与单行水平平移）
        if self.permission.is_none() {
            let mut cur_line: u16 = 0;
            let mut cur_col: u16 = 0;
            for (i, ch) in self.input.chars().enumerate() {
                if i == self.cursor_pos {
                    break;
                }
                if ch == '\n' {
                    cur_line += 1;
                    cur_col = 0;
                } else {
                    cur_col += 1;
                }
            }
            let effective_col = if line_count == 1 {
                let display_start = if self.cursor_pos >= inner_width {
                    self.cursor_pos - inner_width + 1
                } else {
                    0
                };
                (self.cursor_pos - display_start) as u16
            } else {
                cur_col
            };
            let cursor_x = (chunks[1].x + 1 + effective_col)
                .min(chunks[1].x + chunks[1].width.saturating_sub(2));
            let cursor_y = (chunks[1].y + 1 + cur_line)
                .min(chunks[1].y + chunks[1].height.saturating_sub(2));
            frame.set_cursor_position((cursor_x, cursor_y));
        }

        // ── 智能浮层弹出菜单（一级斜杠指令 / 二级模型选择 / 二级提供商选择 / 二级会话恢复） ──
        let (popup_title, popup_items) = self.active_popup_items();
        if !popup_items.is_empty() {
            let total_matches = popup_items.len();
            let max_visible = 8;
            let window_size = total_matches.min(max_visible);
            let popup_height = (window_size + 2) as u16;
            let popup_width = chunks[1].width.min(78);
            let popup_y = chunks[1].y.saturating_sub(popup_height);
            let actual_popup_height =
                popup_height.min(chunks[0].height.saturating_add(popup_height));
            let popup_area = Rect {
                x: chunks[1].x,
                y: popup_y,
                width: popup_width,
                height: actual_popup_height,
            };

            // 清除浮层下方已有绘制，避免文本透视混叠
            frame.render_widget(Clear, popup_area);

            let selected_idx = self
                .slash_popup_index
                .min(total_matches.saturating_sub(1));
            let start_idx = if selected_idx >= window_size {
                selected_idx + 1 - window_size
            } else {
                0
            };
            let end_idx = (start_idx + window_size).min(total_matches);

            let title = format!(
                " {} [{}/{} · Tab/Enter 选择, ↑↓ 切换, Esc 取消] ",
                popup_title,
                selected_idx + 1,
                total_matches
            );
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(title);

            let mut lines = Vec::new();
            for (offset, item) in popup_items[start_idx..end_idx].iter().enumerate() {
                let actual_idx = start_idx + offset;
                let is_selected = actual_idx == selected_idx;
                if is_selected {
                    let mut spans = vec![
                        Span::styled(
                            "> ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{:<26} ", item.name),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if !item.args.is_empty() {
                        spans.push(Span::styled(
                            format!("{:<8} ", item.args),
                            Style::default().fg(Color::LightYellow),
                        ));
                    }
                    spans.push(Span::styled(
                        &item.desc,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                    lines.push(Line::from(spans));
                } else {
                    let mut spans = vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(
                            format!("{:<26} ", item.name),
                            Style::default().fg(Color::Cyan),
                        ),
                    ];
                    if !item.args.is_empty() {
                        spans.push(Span::styled(
                            format!("{:<8} ", item.args),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    spans.push(Span::styled(&item.desc, Style::default().fg(Color::Gray)));
                    lines.push(Line::from(spans));
                }
            }

            let popup_widget = Paragraph::new(lines).block(block);
            frame.render_widget(popup_widget, popup_area);
        }

        // ── 状态栏 ──
        let status = if let Some(prompt) = &self.permission {
            format!(
                "⚠ {} 请求权限 · {}",
                prompt.tool_name,
                prompt.payload.chars().take(80).collect::<String>()
            )
        } else {
            let tools_tag = if self.tools_expanded {
                "[工具详情:展开]"
            } else {
                "[工具详情:折叠]"
            };
            format!(
                "{} · tokens ↑{} ↓{} · {} · {}",
                self.status, self.tokens.0, self.tokens.1, tools_tag, self.conversation_id
            )
        };
        let status_line = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(status_line, chunks[2]);
    }
}
