//! 工具调度执行管线统一协调器（Tool Execution Orchestrator）
//!
//! **职责边界（别按 Codex 的同名类型理解）**：本模块只负责
//! 「批量调度 + 只读并发 + 死循环熔断 + 结果落库/事件广播」。
//! 审批、沙箱选择与沙箱失败重试都在 `execute_single_tool_call` →
//! `execute_tool_with_app` 内完成，不在这里。Codex 的 `ToolOrchestrator`
//! 是审批/沙箱/重试的编排器，两者不是一回事。
//!
//! 单次调用依次经过：
//! 1. 预检查与生命周期 Hook (PreToolUseHooks)
//! 2. 输入 Schema 严格校验 (Input Schema Validation)
//! 3. 权限与子代理策略检查 (Subagent Whitelist & Tool Permissions)
//! 4. 用户交互审批拦截 (User Approval Interception)
//! 5. 核心工具与 MCP 动态分发执行 (Core Execution with Cancellation & Sandbox)
//! 6. 死循环熔断评估 (ToolLoopBreaker Evaluation)
//! 7. 后置审计 Hook 与执行结果/事件规范化 (PostToolUseHooks & Trace Outcomes)

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::agent::cancellation;
use crate::agent::session::SessionEvent;
use crate::agent::tools::{
    cancelled_result_from_call, execute_single_tool_call, is_read_only_tool,
    max_tool_use_concurrency, ToolCallRequest, ToolCallResult,
};
use crate::core::Core;
use nova_protocol::agent::AgentDomainEvent;

/// 工具调用循环熔断动作
pub enum CircuitAction {
    None,
    InjectWarning(String),
    TripAndHalt(String),
}

/// 工具调用死循环熔断器
/// 针对连续相同参数且执行报错/无意义重复的调用进行即时警示或截断，杜绝无脑试错烧穿 token
#[derive(Default)]
pub struct ToolLoopBreaker {
    pub last_signature: Option<String>,
    pub consecutive_same_call_count: usize,
    pub consecutive_failure_count: usize,
}

impl ToolLoopBreaker {
    pub fn new() -> Self {
        Self::default()
    }

    fn signature(name: &str, input: &serde_json::Value) -> String {
        format!("{}:{}", name, input)
    }

    pub fn record_and_evaluate(
        &mut self,
        tool_name: &str,
        input: &serde_json::Value,
        is_error: bool,
    ) -> CircuitAction {
        let sig = Self::signature(tool_name, input);
        if self.last_signature.as_deref() == Some(&sig) {
            self.consecutive_same_call_count += 1;
            if is_error {
                self.consecutive_failure_count += 1;
            } else {
                self.consecutive_failure_count = 0;
            }
        } else {
            self.last_signature = Some(sig);
            self.consecutive_same_call_count = 1;
            self.consecutive_failure_count = if is_error { 1 } else { 0 };
        }

        if self.consecutive_failure_count >= 5 {
            CircuitAction::TripAndHalt(format!(
                "[Circuit Breaker Tripped]: Tool '{}' has failed 5 consecutive times with identical parameters. Halting execution to prevent infinite loop.",
                tool_name
            ))
        } else if self.consecutive_failure_count >= 3 {
            CircuitAction::InjectWarning(format!(
                "\n\n[Nova Circuit Breaker Warning]: Tool '{}' has failed {} consecutive times with the exact same input. You MUST try a different approach, verify path/arguments, or explain the difficulty to the user instead of retrying this call.",
                tool_name, self.consecutive_failure_count
            ))
        } else if self.consecutive_same_call_count >= 6 {
            CircuitAction::InjectWarning(format!(
                "\n\n[Nova Circuit Breaker Warning]: Tool '{}' has been invoked {} consecutive times with identical parameters. Please progress to the next step or synthesize results.",
                tool_name, self.consecutive_same_call_count
            ))
        } else {
            CircuitAction::None
        }
    }
}

/// 单轮执行中工具调度的全局上下文对象。
#[derive(Clone)]
pub struct ToolExecutionContext {
    pub core: Core,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub trace_id: Option<String>,
    pub cancellation_token: CancellationToken,
}

impl ToolExecutionContext {
    pub fn new(core: Core, conversation_id: Option<String>, turn_id: Option<String>) -> Self {
        let token = cancellation::get_token(conversation_id.as_deref());
        let trace_id = core.active_trace_id(conversation_id.as_deref(), turn_id.as_deref());
        Self {
            core,
            conversation_id,
            turn_id,
            trace_id,
            cancellation_token: token,
        }
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}

pub struct ToolOrchestrator;

impl ToolOrchestrator {
    /// 执行单个工具调用（含预检查、审批、执行与死循环熔断判定）。
    pub async fn run_single(
        ctx: &ToolExecutionContext,
        call: ToolCallRequest,
        mut loop_breaker: Option<&mut ToolLoopBreaker>,
    ) -> ToolCallResult {
        let start_time = Instant::now();
        let call_input = call.input.clone();
        let call_name = call.name.clone();

        let mut executed = execute_single_tool_call(
            &ctx.core,
            ctx.conversation_id.as_deref(),
            call,
        )
        .await;

        let duration_ms = start_time.elapsed().as_millis() as u64;

        // 死循环熔断评估（若传入 breaker）
        if let Some(ref mut breaker) = loop_breaker {
            match breaker.record_and_evaluate(&call_name, &call_input, executed.is_error) {
                CircuitAction::None => {}
                CircuitAction::InjectWarning(warning) => {
                    tracing::warn!(
                        tool = %call_name,
                        "ToolLoopBreaker injected warning for repeated execution"
                    );
                    executed.output.push_str(&warning);
                }
                CircuitAction::TripAndHalt(halt_msg) => {
                    tracing::error!(
                        tool = %call_name,
                        "ToolLoopBreaker tripped and halted execution"
                    );
                    executed.output = halt_msg;
                    executed.is_error = true;
                    executed.prevent_continuation = true;
                    executed.stop_reason = Some("tool_loop_circuit_trip".to_string());
                }
            }
        }

        // 统一事件落库与总线广播
        if let (Some(conv_id), Some(turn_id)) = (ctx.conversation_id.as_deref(), ctx.turn_id.as_deref()) {
            let trace_id = ctx
                .trace_id
                .clone()
                .or_else(|| ctx.core.active_trace_id(Some(conv_id), Some(turn_id)));
            let now_ms = chrono::Utc::now().timestamp_millis();
            let _ = crate::agent::session::append_event(
                &ctx.core,
                conv_id,
                Some(turn_id),
                &SessionEvent::ToolResult {
                    call_id: executed.id.clone(),
                    tool_name: executed.name.clone(),
                    output: executed.output.clone(),
                    is_error: executed.is_error,
                    started_at: now_ms.saturating_sub(duration_ms as i64),
                    finished_at: now_ms,
                    trace_id: trace_id.clone(),
                },
            )
            .await;

            ctx.core.emit_agent(AgentDomainEvent::ToolCallCompleted {
                turn_id: turn_id.to_string(),
                call_id: executed.id.clone(),
                tool_name: executed.name.clone(),
                is_error: executed.is_error,
                output: executed.output.clone(),
                duration_ms,
                trace_id,
            });
        }

        executed
    }

    /// 把熔断动作落到结果上（只读批次此前只记录不施加，导致重复失败永远不计入）。
    fn apply_circuit_action(result: &mut ToolCallResult, action: CircuitAction) {
        match action {
            CircuitAction::None => {}
            CircuitAction::InjectWarning(warning) => result.output.push_str(&warning),
            CircuitAction::TripAndHalt(halt_msg) => {
                result.output = halt_msg;
                result.is_error = true;
                result.prevent_continuation = true;
                result.stop_reason = Some("tool_loop_circuit_trip".to_string());
            }
        }
    }

    /// 执行只读并发批次
    async fn execute_read_only_queue(
        ctx: &ToolExecutionContext,
        calls: Vec<ToolCallRequest>,
    ) -> Vec<ToolCallResult> {
        let total = calls.len();
        if total == 0 {
            return Vec::new();
        }

        let mut queue: VecDeque<(usize, ToolCallRequest)> = calls.into_iter().enumerate().collect();
        let mut in_flight: BTreeMap<usize, ToolCallRequest> = BTreeMap::new();
        let mut results_by_index: BTreeMap<usize, ToolCallResult> = BTreeMap::new();
        let mut tasks: JoinSet<(usize, ToolCallResult)> = JoinSet::new();
        let mut cancellation_reason: Option<String> = None;
        let max_concurrency = max_tool_use_concurrency();

        while !queue.is_empty() || !tasks.is_empty() {
            if cancellation::is_cancelled(ctx.conversation_id.as_deref()) {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                cancellation_reason = Some("cancelled".into());
                break;
            }

            while tasks.len() < max_concurrency && !queue.is_empty() {
                let Some((index, call)) = queue.pop_front() else {
                    break;
                };

                let sub_ctx = ctx.clone();
                in_flight.insert(index, call.clone());
                tasks.spawn(async move {
                    let result = Self::run_single(&sub_ctx, call, None).await;
                    (index, result)
                });
            }

            let maybe_joined = tokio::select! {
                biased;
                _ = ctx.cancellation_token.cancelled() => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    cancellation_reason = Some("cancelled".into());
                    break;
                }
                joined = tasks.join_next() => joined,
            };

            let Some(joined) = maybe_joined else {
                break;
            };

            if let Ok((index, result)) = joined {
                in_flight.remove(&index);
                results_by_index.insert(index, result);
            }
        }

        if let Some(reason) = cancellation_reason {
            for (index, call) in in_flight.into_iter() {
                results_by_index
                    .entry(index)
                    .or_insert_with(|| cancelled_result_from_call(call, &reason));
            }
            while let Some((index, call)) = queue.pop_front() {
                results_by_index
                    .entry(index)
                    .or_insert_with(|| cancelled_result_from_call(call, &reason));
            }
        } else {
            for (index, call) in in_flight.into_iter() {
                results_by_index.entry(index).or_insert_with(|| {
                    cancelled_result_from_call(call, "cancelled: read-only task aborted")
                });
            }
            while let Some((index, call)) = queue.pop_front() {
                results_by_index.entry(index).or_insert_with(|| {
                    cancelled_result_from_call(call, "cancelled: read-only task not executed")
                });
            }
        }

        let mut ordered_results = Vec::with_capacity(total);
        for index in 0..total {
            if let Some(result) = results_by_index.remove(&index) {
                ordered_results.push(result);
            }
        }
        ordered_results
    }

    /// 统一管线执行整个批次的工具调用：
    /// 自动划分只读工具并发流与串行有副作用工具流，并逐个贯穿完整中间件。
    pub async fn run_batch(
        ctx: &ToolExecutionContext,
        calls: Vec<ToolCallRequest>,
        loop_breaker: &mut ToolLoopBreaker,
    ) -> Vec<ToolCallResult> {
        let mut results = Vec::with_capacity(calls.len());
        let mut read_only_batch = Vec::new();

        for call in calls {
            if is_read_only_tool(&call.name) {
                read_only_batch.push(call);
                continue;
            }

            // 遇到写操作工具，先冲刷之前累积的只读并发任务
            if !read_only_batch.is_empty() {
                let drained = std::mem::take(&mut read_only_batch);
                let ro_results = Self::execute_read_only_queue(ctx, drained).await;
                for mut res in ro_results {
                    let action =
                        loop_breaker.record_and_evaluate(&res.name, &res.input, res.is_error);
                    Self::apply_circuit_action(&mut res, action);
                    let stop = res.prevent_continuation;
                    results.push(res);
                    if stop {
                        return results;
                    }
                }
            }

            // 串行执行有副作用工具
            if cancellation::is_cancelled(ctx.conversation_id.as_deref()) {
                results.push(cancelled_result_from_call(
                    call,
                    "Tool execution cancelled before run",
                ));
                break;
            }

            let res = Self::run_single(ctx, call, Some(loop_breaker)).await;
            let should_stop = res.prevent_continuation;
            results.push(res);

            if should_stop {
                break;
            }
        }

        // 冲刷末尾剩余的只读批次
        if !read_only_batch.is_empty() {
            let drained = std::mem::take(&mut read_only_batch);
            let ro_results = Self::execute_read_only_queue(ctx, drained).await;
            for mut res in ro_results {
                let action = loop_breaker.record_and_evaluate(&res.name, &res.input, res.is_error);
                Self::apply_circuit_action(&mut res, action);
                results.push(res);
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_loop_breaker_trips_at_5_failures() {
        let mut breaker = ToolLoopBreaker::new();
        let tool = "test_tool";
        let input = json!({"arg": 1});
        for _ in 0..4 {
            let action = breaker.record_and_evaluate(tool, &input, true);
            match action {
                CircuitAction::None | CircuitAction::InjectWarning(_) => {}
                CircuitAction::TripAndHalt(_) => panic!("Should not trip before 5th failure"),
            }
        }
        let action = breaker.record_and_evaluate(tool, &input, true);
        match action {
            CircuitAction::TripAndHalt(_) => {}
            _ => panic!("Must trip at 5th failure"),
        }
    }
}
