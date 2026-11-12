use ts_rs::TS;
use serde::{Deserialize, Serialize};

/// 现代化 Agent 领域事件系统。
/// 后端作为唯一真实信源（Single Source of Truth），向前端推送确定性的高阶事件。
/// 前端仅需单向订阅，无需拼装消息状态或二次计算。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
#[ts(export, export_to = "protocol.ts")]
pub enum AgentDomainEvent {
    /// 回合启动
    TurnStarted {
        turn_id: String,
        conversation_id: String,
        #[ts(type = "number")]
        timestamp: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 认知状态机流转事件
    StateChanged {
        turn_id: String,
        state: crate::agent::CognitiveState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 模型思考/推理流式增量（thinking / reasoning delta）
    ThinkingDelta {
        turn_id: String,
        delta: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 模型回答正文流式增量
    TextDelta {
        turn_id: String,
        delta: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 模型发起工具调用请求（参数已完成解析与校验）
    ToolCallRequested {
        turn_id: String,
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 工具执行完成（包含执行输出与耗时）
    ToolCallCompleted {
        turn_id: String,
        call_id: String,
        tool_name: String,
        is_error: bool,
        output: String,
        #[ts(type = "number")]
        duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// Token 用量与成本更新
    TokenUsageUpdate {
        turn_id: String,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_creation_tokens: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 触发安全审批拦截请求
    PermissionRequested {
        turn_id: String,
        request_id: String,
        tool_name: String,
        payload: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 回合成功完成
    TurnFinished {
        turn_id: String,
        stop_reason: String,
        total_tokens: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 回合异常终止
    TurnError {
        turn_id: String,
        error: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_info: Option<crate::errors::NovaErrorInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 用户补充 steer 消息已在工具边界注入当前回合
    TurnSteered {
        turn_id: String,
        conversation_id: String,
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
    /// 用户消息已进入顺序队列，等待当前回合完成后执行
    TurnQueued {
        turn_id: String,
        conversation_id: String,
        queue_length: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
}

impl AgentDomainEvent {
    pub fn turn_id(&self) -> &str {
        match self {
            Self::TurnStarted { turn_id, .. }
            | Self::StateChanged { turn_id, .. }
            | Self::ThinkingDelta { turn_id, .. }
            | Self::TextDelta { turn_id, .. }
            | Self::ToolCallRequested { turn_id, .. }
            | Self::ToolCallCompleted { turn_id, .. }
            | Self::TokenUsageUpdate { turn_id, .. }
            | Self::PermissionRequested { turn_id, .. }
            | Self::TurnFinished { turn_id, .. }
            | Self::TurnError { turn_id, .. }
            | Self::TurnSteered { turn_id, .. }
            | Self::TurnQueued { turn_id, .. } => turn_id,
        }
    }

    pub fn conversation_id_hint(&self) -> Option<&str> {
        match self {
            Self::TurnStarted {
                conversation_id, ..
            }
            | Self::TurnSteered {
                conversation_id, ..
            }
            | Self::TurnQueued {
                conversation_id, ..
            } => Some(conversation_id.as_str()),
            _ => None,
        }
    }

    pub fn trace_id(&self) -> Option<&str> {
        match self {
            Self::TurnStarted { trace_id, .. }
            | Self::StateChanged { trace_id, .. }
            | Self::ThinkingDelta { trace_id, .. }
            | Self::TextDelta { trace_id, .. }
            | Self::ToolCallRequested { trace_id, .. }
            | Self::ToolCallCompleted { trace_id, .. }
            | Self::TokenUsageUpdate { trace_id, .. }
            | Self::PermissionRequested { trace_id, .. }
            | Self::TurnFinished { trace_id, .. }
            | Self::TurnError { trace_id, .. }
            | Self::TurnSteered { trace_id, .. }
            | Self::TurnQueued { trace_id, .. } => trace_id.as_deref(),
        }
    }

    pub fn set_trace_id(&mut self, value: String) {
        let slot = match self {
            Self::TurnStarted { trace_id, .. }
            | Self::StateChanged { trace_id, .. }
            | Self::ThinkingDelta { trace_id, .. }
            | Self::TextDelta { trace_id, .. }
            | Self::ToolCallRequested { trace_id, .. }
            | Self::ToolCallCompleted { trace_id, .. }
            | Self::TokenUsageUpdate { trace_id, .. }
            | Self::PermissionRequested { trace_id, .. }
            | Self::TurnFinished { trace_id, .. }
            | Self::TurnError { trace_id, .. }
            | Self::TurnSteered { trace_id, .. }
            | Self::TurnQueued { trace_id, .. } => trace_id,
        };
        *slot = Some(value);
    }
}

/// 回合提交并发策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "protocol.ts")]
pub enum TurnSubmitMode {
    /// 无活跃回合则立即启动；若有活跃回合则作为 steer 注入下一工具批边界。
    #[default]
    #[serde(alias = "steer")]
    StartOrSteer,
    /// 若有活跃回合则进入顺序队列，等待前序回合结束后依次执行。
    Queue,
}

/// 回合提交的同步仲裁结果类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "protocol.ts")]
pub enum TurnSubmitOutcomeKind {
    Started,
    Steered,
    Queued,
}

/// `send_modern_agent_turn` 的同步仲裁回包。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "protocol.ts")]
pub struct AgentTurnSubmitOutcome {
    pub outcome: TurnSubmitOutcomeKind,
    pub turn_id: String,
    pub queue_length: u32,
}



/// 严格的 Agent 认知状态机（Cognitive State Machine）
/// 严格跟踪从“意图理解 -> 工具调用 -> 验证自愈 -> 任务完成”的每一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
#[ts(export, export_to = "protocol.ts")]
pub enum CognitiveState {
    /// 空闲中，等待用户指令
    #[default]
    Idle,
    /// 上下文装配中（Prompt Caching 对齐、历史重构、符号大纲）
    AssemblingContext,
    /// 模型推理中（流式接收 Thinking 与 Text）
    ModelInference,
    /// 等待敏感操作人工审批
    AwaitingApproval,
    /// 工具执行中
    ExecutingTool,
    /// 代码修改后的编译/语法自愈验证中
    VerifyingWorkspace,
    /// 诊断失败，正在自反思并向模型回灌编译器错误
    Reflecting,
    /// 当前轮次完成
    TurnComplete,
    /// 异常中断
    Failed,
}


/// 审批策略：与操作风险正交的全局开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "protocol.ts")]
pub enum ApprovalPolicy {
    /// 凡产生权限描述符的操作一律审批（Forbidden 仍直接拒绝）。
    AlwaysAsk,
    /// 仅 Risky 操作审批，Safe 直接放行。
    #[default]
    OnRequest,
    /// 从不审批：Risky 也放行（Forbidden 仍直接拒绝）。
    /// Auto 迭代模式当轮等效于此策略。
    Never,
}

impl ApprovalPolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "always_ask" | "alwaysask" => Some(Self::AlwaysAsk),
            "on_request" | "onrequest" => Some(Self::OnRequest),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    pub fn as_setting_str(&self) -> &'static str {
        match self {
            Self::AlwaysAsk => "always_ask",
            Self::OnRequest => "on_request",
            Self::Never => "never",
        }
    }
}
