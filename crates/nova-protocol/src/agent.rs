use serde::{Deserialize, Serialize};

/// 现代化 Agent 领域事件系统。
/// 后端作为唯一真实信源（Single Source of Truth），向前端推送确定性的高阶事件。
/// 前端仅需单向订阅，无需拼装消息状态或二次计算。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum AgentDomainEvent {
    /// 回合启动
    TurnStarted {
        turn_id: String,
        conversation_id: String,
        timestamp: i64,
    },
    /// 认知状态机流转事件
    StateChanged {
        turn_id: String,
        state: crate::agent::CognitiveState,
    },
    /// 模型思考/推理流式增量（thinking / reasoning delta）
    ThinkingDelta {
        turn_id: String,
        delta: String,
    },
    /// 模型回答正文流式增量
    TextDelta {
        turn_id: String,
        delta: String,
    },
    /// 模型发起工具调用请求（参数已完成解析与校验）
    ToolCallRequested {
        turn_id: String,
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// 工具执行完成（包含执行输出与耗时）
    ToolCallCompleted {
        turn_id: String,
        call_id: String,
        tool_name: String,
        is_error: bool,
        output: String,
        duration_ms: u64,
    },
    /// Token 用量与成本更新
    TokenUsageUpdate {
        turn_id: String,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_creation_tokens: u32,
    },
    /// 触发安全审批拦截请求
    PermissionRequested {
        turn_id: String,
        request_id: String,
        tool_name: String,
        payload: String,
    },
    /// 回合成功完成
    TurnFinished {
        turn_id: String,
        stop_reason: String,
        total_tokens: u32,
    },
    /// 回合异常终止
    TurnError {
        turn_id: String,
        error: String,
    },
}



/// 严格的 Agent 认知状态机（Cognitive State Machine）
/// 严格跟踪从“意图理解 -> 工具调用 -> 验证自愈 -> 任务完成”的每一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
