use serde::{Deserialize, Serialize};

/// 严格的 Agent 认知状态机（Cognitive State Machine）
/// 严格跟踪从“意图理解 -> 工具调用 -> 验证自愈 -> 任务完成”的每一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveState {
    /// 空闲中，等待用户指令
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

impl Default for CognitiveState {
    fn default() -> Self {
        Self::Idle
    }
}
