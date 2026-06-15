#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    // 回合自然完成（无进一步用户交互需求）。
    Completed,
    // 回合等待用户补充输入后再继续。
    NeedsUserInput,
    // 回合被用户主动取消。
    Cancelled,
    // 回合被 stop hook 明确阻断。
    StopHookPrevented,
    // 回合因 provider 错误或内部一致性校验失败而终止。
    Error,
}

impl TurnState {
    pub fn as_event_state(self) -> &'static str {
        // 将内部状态映射为前端事件层约定字符串。
        match self {
            // Completed -> completed。
            Self::Completed => "completed",
            // NeedsUserInput -> needs_user_input。
            Self::NeedsUserInput => "needs_user_input",
            // Cancelled -> cancelled。
            Self::Cancelled => "cancelled",
            // StopHookPrevented -> stop_hook_prevented。
            Self::StopHookPrevented => "stop_hook_prevented",
            // Error -> error。
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    // 终止原因文本（供日志与前端展示）。
    pub stop_reason: String,
    // 回合状态枚举（供流程判断与事件映射）。
    pub turn_state: TurnState,
}

impl TurnOutcome {
    pub fn completed(stop_reason: impl Into<String>) -> Self {
        // 构造 completed 结果并保存 stop_reason。
        Self {
            // 将入参统一转换为 String。
            stop_reason: stop_reason.into(),
            // 标记为 Completed。
            turn_state: TurnState::Completed,
        }
    }

    pub fn needs_user_input() -> Self {
        // 构造 needs_user_input 结果。
        Self {
            // 固定停止原因为 needs_user_input。
            stop_reason: "needs_user_input".to_string(),
            // 标记为 NeedsUserInput。
            turn_state: TurnState::NeedsUserInput,
        }
    }

    pub fn cancelled() -> Self {
        // 构造 cancelled 结果。
        Self {
            // 固定停止原因为 cancelled。
            stop_reason: "cancelled".to_string(),
            // 标记为 Cancelled。
            turn_state: TurnState::Cancelled,
        }
    }

    pub fn stop_hook_prevented(stop_reason: impl Into<String>) -> Self {
        // 构造 stop_hook_prevented 结果并保存 stop_reason。
        Self {
            // 将入参统一转换为 String。
            stop_reason: stop_reason.into(),
            // 标记为 StopHookPrevented。
            turn_state: TurnState::StopHookPrevented,
        }
    }

    pub fn error(stop_reason: impl Into<String>) -> Self {
        // 构造 error 结果并保存 stop_reason（含错误详情，供 Err() 返回）。
        Self {
            // 将入参统一转换为 String。
            stop_reason: stop_reason.into(),
            // 标记为 Error。
            turn_state: TurnState::Error,
        }
    }
}
