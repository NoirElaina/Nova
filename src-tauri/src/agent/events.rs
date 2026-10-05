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
        state: crate::agent::state::CognitiveState,
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
    /// 触发代码验证自愈环节（如 syntax check / linter / compiler）
    VerificationStarted {
        turn_id: String,
        target: String,
    },
    /// 代码验证完成
    VerificationCompleted {
        turn_id: String,
        target: String,
        passed: bool,
        feedback: Option<String>,
    },
    /// Token 用量与成本更新
    TokenUsageUpdate {
        turn_id: String,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_creation_tokens: u32,
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
