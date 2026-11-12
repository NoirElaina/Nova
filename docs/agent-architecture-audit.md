# Nova × Codex Agent 架构审计

> 基准：`D:/ref/codex`（124 crates）
> 对象：`D:/Issue/MicrosoftVSCode/tauri/Nova`（4 crates + src-tauri，约 11k 行 Rust）
> 方法：逐文件读取关键函数体，所有结论附 `文件:行号`
> 证据强度：**[实读]** 读过函数体 · **[签名]** 只读了函数签名/结构 · **[统计]** 只有规模数字

---

## 0. 分析框架

Agent 架构不是"有多少工具"的问题，而是七个可判定的维度。任何一个维度缺失，都会在特定场景下变成可观察的故障：

| # | 维度 | 判定的问题 | 缺失时的表现 |
|---|---|---|---|
| 1 | 执行模型 | turn 的边界是什么，谁拥有它 | 无法取消、无法排队、无法恢复 |
| 2 | 上下文构建 | 模型每一步看到什么 | 长对话膨胀、状态过期、模态不匹配 |
| 3 | 模型交互 | 请求/流/重试的控制流 | 限流即失败、超限即崩 |
| 4 | 工具调度 | 选择 → 审批 → 沙箱 → 执行 → 回填 | 逻辑散落、语义不一致 |
| 5 | 状态与事件 | 如何记录、如何重建 | 丢一条事件即状态错乱 |
| 6 | 并发与取消 | 多 turn、取消传播 | 连点报错、停止不生效 |
| 7 | 失败处理 | 分类、重试、降级 | 所有错误都一样，无法决策 |

以下按这七维展开。

---

## 1. 执行模型

### Nova

入口在 `crates/nova-core/src/agent/mod.rs:18`：

```rust
pub async fn send_modern_agent_turn(core, conversation_id, prompt, attachments) -> Result<(), String> {
    if !cancellation::begin_turn(Some(&conversation_id)) {
        return Err("该会话已有正在进行的回复，请等待其完成或先停止。".to_string());
    }
    let cancel_token = cancellation::get_token(Some(&conversation_id));
    let engine = engine::AgentEngine::new(core);
    let result = engine.execute_turn(&conversation_id, &prompt, attachments, cancel_token).await;
    cancellation::finish_turn(Some(&conversation_id));
    result.map(|_| ())
}
```

**[实读]** 三个事实：

1. **并发是拒绝，不是排队**：第二个 turn 直接 `Err`
2. **turn 的身份是 `conversation_id` 字符串**，没有独立的 Turn 对象
3. **`AgentEngine::new(core)` 每轮新建**，引擎不持有会话状态

`execute_turn` 从 `engine.rs:128` 一直到 `:742`，**614 行单函数**。已读段（128-190、300-375）显示内部按顺序做：

```
TurnStarted + StateChanged::AssemblingContext   (:135-145)
落库 UserMessage / TurnStart                     (:148 起)
附件处理（image base64 / document 提取）          (:150-200)
 ┌ loop
 │  mid-turn 自动压缩（compact_outcome.applied）  (:300-315)
 │  StateChanged::ModelInference                 (:307)
 │  client.send_request(...)                     (:315)
 │  prompt_too_long → 反应式压缩重试              (:325-360)
 │  工具执行 + ToolLoopBreaker
 └
TurnEnd / StateChanged::Final
```

**[实读]** `ToolLoopBreaker` 存在且有测试（`:778 test_tool_loop_breaker_injects_warning_at_3_failures`、`:805 ...halts_at_5_failures`）：同一工具同参数失败 3 次注入警告、5 次熔断。**这是 Codex 没有的机制**。

### Codex

`core/src/session/turn.rs:217`：

```rust
pub(crate) async fn run_turn(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    mut input: Vec<TurnInput>,
    mcp_startup_requirements: &mut McpStartupRequirements,
    prewarmed_client_session: Option<ModelClientSession>,
    cancellation_token: CancellationToken,
) -> CodexResult<Option<String>>
```

**[实读]** 三层对象：`Session`（跨 turn 存活）/ `TurnContext`（本轮配置）/ `StepContext`（采样步）。函数拆成 7 段，各自单一职责：

| 函数 | 行 | 职责 |
|---|---|---|
| `run_hooks_and_record_inputs` | 1038 | hook 结果 + 输入落库 |
| `run_pre_sampling_compact` | 1548 | 预采样压缩 |
| `run_auto_compact` | 1683 | 自动压缩 |
| `run_sampling_request` | 1891 | 请求 + 重试循环 |
| `prepare_tool_recommendations` | 2097 | 工具推荐 |
| `built_tools` | 2144 | 工具集构建 |

**[实读]** `run_turn` 开头还做了 Nova 没有的事：
- 音频输入与模型模态不匹配时**提前拒绝并落库**（`:224-240`）
- `AutoCompactionDisabled` 扩展检查（`:240-252`）
- `guardian::check_pending_guardian_input`（`:254`）
- `drain_async_hook_results(before_user_prompt: true)`（`:257`）——上一轮异步 hook 结果在本轮用户输入前落地

### 差异与影响

| | Nova | Codex |
|---|---|---|
| turn 对象 | 仅 `conversation_id` | Session / TurnContext / StepContext |
| 并发 | 拒绝 | ThreadManager 排队 |
| 单函数体量 | 614 行 | 最大 430 行（run_turn 217-1038 内含分支） |
| 步级上下文 | 无 | `StepContext`（含 `sampled_through_sequence`、`tool_router`、`preempt`） |

**影响**：
- 无法"取消后接着跑"—— 没有 step 边界可恢复
- 无法按步统计（`turn_timing/` 存在的前提是 StepContext）
- 连点两次发送弹错误，是并发模型的直接后果

---

## 2. 上下文构建

### Nova

`crates/nova-core/src/agent/context/assembler.rs:21`：

```rust
pub async fn assemble(core, conversation_id, mut history: Vec<Message>) -> Result<Vec<Message>, String> {
    compact::drop_orphan_tool_blocks(&mut history);            // 1 清洗孤儿 tool 块
    strip_stale_workspace_contexts(&mut history);              // 2 剔除旧工作区快照
    let snapshot = build_workspace_snapshot(&root);            // 3 重建快照
    inject_fresh_workspace_context(&mut history, &snapshot);   //   文本注入
    normalize_role_alternation(history)                        // 4 角色交替
}
```

**[实读]** 四步，本质是**文本注入**：工作区状态被序列化成一段文字塞进消息。

函数的其余部分（`:50 strip_context_text`、`:64 strip_stale_workspace_contexts`、`:91 build_workspace_snapshot`、`:184 inject_fresh_workspace_context`、`:211 normalize_role_alternation`）全部服务于"怎么拼这段文字"。

### Codex

`core/src/context_manager/`，**7312 行**，11 个文件：

```
history.rs            历史（含 input_sequence 序号）
normalize.rs          规范化
sampling.rs           采样视图
updates.rs            增量更新
image_availability.rs 图片可用性
history_user_authorization.rs
history_rejected_audio.rs
```

**[实读]** `run_sampling_request:1960` 的用法：

```rust
let mut prompt_input = history
    .for_prompt(&step_context.settings.model_info.input_modalities, sess.services.image_store.as_ref())
    .await;
sess.services.executed_tool_calls.attach_to_prompt(&mut prompt_input, &mut executed_tool_calls_by_output);
```

三个关键点：
1. `for_prompt(modalities, image_store)` —— **按模型支持的模态过滤**内容，图片走 store 引用而非 base64 内联
2. `attach_to_prompt` —— 工具结果有**专门的注入通道**，不是 append 到 messages
3. `input_sequence()` —— 每条历史有单调序号，重试时能定位"从哪条开始重放"

### 差异与影响

| | Nova | Codex |
|---|---|---|
| 形态 | `Vec<Message>` + 文本拼接 | `ContextManager` + 类型化视图 |
| 工作区状态 | 序列化成文字注入最新一轮 | 由 `updates.rs` 做增量更新 |
| 模态 | 手动组装 attachments | `for_prompt` 按 `input_modalities` 过滤 |
| 序号 | 无 | `input_sequence()` |
| 工具结果 | append messages | `attach_to_prompt` 专用通道 |

**影响**：Nova 的工作区快照是"上一轮的文字残留"，模型可能看到过期路径；Codex 的上下文是每步重建的。

---

## 3. 模型交互与重试

### Nova

`crates/nova-core/src/provider/stream_runner.rs:215 run_streaming`，865 行。

**[实读]** `engine.rs:325` 的重试：

```rust
Err(err) => {
    let err_msg = err.message.clone();
    if crate::agent::capabilities::compact::is_prompt_too_long_error(&err_msg) {
        // 反应式压缩 → 重发一次
    } else { return self.handle_turn_error(...).await; }
}
```

**控制流建立在字符串匹配上**：`is_prompt_too_long_error(&err_msg)` 判断是不是上下文超限。

`stream_runner.rs` 的失败分支（`:290`）同样是字符串判断：

```rust
|| err_str.contains("stream reading error")
```

### Codex

**[实读]** `run_sampling_request:1891` 内部：

```rust
let max_retries = turn_context.provider.info().stream_max_retries();
let mut retry_state = ResponsesStreamRetryState::default();
let mut initial_input = Some(input);
loop {
    let first_attempt = initial_input.is_some();
    let history = if let Some(input) = initial_input.take() { input }
                  else { sess.clone_history().await };
    ...
}
```

- **重试次数来自 provider 配置**（`stream_max_retries()`），不是硬编码
- **`ResponsesStreamRetryState`** 是类型化状态机
- **`first_attempt` + `initial_input.take()`**：首次用传入输入，重试用克隆历史 —— 避免输入被消费两次

**[实读]** 错误类型定义在 `app-server-protocol/src/protocol/v2/shared.rs`，`CodexErr` **34 个变体**，含：

```
ContextWindowExceeded  SessionBudgetExceeded  UsageLimitExceeded
RateLimitExceeded      ServerOverloaded        ModelNotFound
PolicyCapacityUnavailable  CyberPolicy        MisalignmentPolicyViolation
```

Nova 对应物：`crates` 内 `Result<T, String>` **304 处**，自定义错误枚举 **2 个**（都在 `tools/apply_patch/`）。

### 差异与影响

**这不是代码风格差异，是控制流基础差异。**

字符串错误带来的连锁后果：

| 想做 | Nova 现状 | Codex |
|---|---|---|
| 限流退避 | 只能匹配 "rate limit" 字样 | `RateLimitExceeded` 直接分支 |
| 上下文超限 → 压缩 | `is_prompt_too_long_error` 字符串 | `ContextWindowExceeded` |
| 超预算 → 终止 | 无法区分 | `SessionBudgetExceeded` |
| 传给客户端 | 一个 `TurnError { error: String }` | 34 种结构化错误 |

`handle_turn_error`（`engine.rs:742`，**16 行**）是唯一出口：

```rust
async fn handle_turn_error(&self, conversation_id, turn_id, err_msg: &str) -> Result<TurnOutcome, String> {
    append_event(SessionEvent::TurnEnd { stop_reason: Some("error") });
    emit(StateChanged::Failed);
    emit(TurnError { error: err_msg.to_string() });
    Err(err_msg.to_string())
}
```

所有失败——网络断、限流、超限、鉴权、工具崩——都在这里汇成同一件事。

---

## 4. 工具调度（最大结构差异）

### Nova

**[签名]** `agent/tools/mod.rs:708 get_available_tools()` → `global_tool_registry().list_all_tools()`。

**[实读]** 但 Nova 有 Codex 没有的过滤机制：

```rust
// mod.rs:720  渐进披露
pub(crate) fn deferred_tool_definitions() -> Vec<Tool>   // ToolDisclosure::Deferred
pub(crate) const LOAD_TOOL_NAME: &str = "LoadTool";

// mod.rs:737  按 agent 过滤
pub fn get_available_tools_for_agent(core: &Core, conversation_id: Option<&str>) -> Vec<Tool> {
    // 分支会话（:branch:）：零工具
    // 子代理（:sub:）：只读白名单，无 MCP/Task
    // 未挂 bundle：全部工具
}
```

**审批与沙箱是各工具自己调**：`bash.rs` 里 `enforce_tool_permission(...)` → 按结果决定 `wrap_shell` 用哪档沙箱。

### Codex

`core/src/tools/orchestrator.rs`，**[实读]** 文件头注释就是设计声明：

```rust
//! Coordinates tool approval, sandbox selection, and retries after sandbox denial.
//! Guardian approval applies to the reviewed authority; escalating a sandboxed
//! attempt requires a fresh review of the retry.

pub(crate) struct ToolOrchestrator;
pub(crate) struct OrchestratorRunResult<Out> { ... }
async fn run_attempt<Rq, Out, T>(...)      // :54
pub async fn run<Rq, Out, T>(...)           // :125
```

**[实读]** 它引用的模块说明了一切：

```
tools/approvals.rs      ApprovalAction / ApprovalContext / ApprovalResolutionSource
tools/sandboxing.rs     SandboxAttempt / SandboxOverride / ToolCtx / ToolRuntime
tools/network_approval  NetworkApprovalSpec / begin_network_approval / finish_deferred_network_approval
tools/sandboxing.rs     ensure_native_sandbox / sandbox_override_for_first_attempt
guardian               GuardianReviewContext
```

**一个泛型 `run` 统一处理**：审批 → 选沙箱 → 执行 → 拒绝后重新审查 → 重试。所有工具（`apply_patch`、`unified_exec`、`mcp`…）走同一条路径。

### 差异与影响

| | Nova | Codex |
|---|---|---|
| 调度入口 | 每个工具自己实现 | `ToolOrchestrator::run` 泛型 |
| 审批点 | 工具内调 `enforce_tool_permission` | orchestrator 统一 |
| 沙箱重试 | `bash` 工具内有 `request_sandbox_retry` | orchestrator 内 + Guardian 复审 |
| 网络审批 | 无 | `network_approval` 模块 |
| 并行执行 | 无 | `tools/parallel.rs` |

**影响**：Nova 加一个新工具要重写一遍审批+沙箱+错误处理；Codex 只需实现 `ToolRuntime`。

**同时要承认**：Nova 的 `ToolDisclosure::Deferred` + `LoadTool` 渐进披露是 Codex 没有的等价机制（Codex 用 `tool_search`），这块不是差距。

---

## 5. 状态与事件

### Nova

**[实读]** `agent/session/projection.rs`（619 行）提供四个投影：

```rust
:176 render_ui_history(events)           → Vec<HistoryMessage>
:388 render_tool_logs(events)            → Vec<HistoryToolExecution>
:451 reconstruct_model_context(events)   → Vec<Message>
:488 render_turn_traces(events)          → Vec<TurnTrace>
```

事件源是 `session_events` 表（`session/history.rs`，29 处 SQL 调用），append-only。

**修正一个此前的误判**：Nova **有**从事件重建状态的能力。缺的不是投影，是**把投影暴露成"客户端可调用的重放接口"**。前端一旦丢事件，没有 RPC 能让它"拉取最新状态并收敛"。

### Codex

**[实读]** `app-server-protocol/src/protocol/`：
- `thread_history.rs` **6047 行**（历史）
- `thread_history_projection.rs`（投影）
- `event_mapping.rs`（内部事件 → 协议 Item 的映射）
- `item_builders.rs`（Item 构造）
- `v2/item.rs` 1964 行（**Item 有 id + status 的生命周期**）

**[统计]** `state/` crate 另有 `audit.rs` / `log_db.rs` / `migrations.rs`。

### 差异与影响

```rust
// Nova：事件 = "发生过什么"
AgentDomainEvent::ToolCallCompleted { turn_id, call_id, tool_name, output, duration_ms }
    → serde → 前端顺序消费

// Codex：事件 = "有哪些 Item"
Item { id, kind: ToolCall, status: Completed, .. }
    → 可 upsert、重复推送幂等、可整体重放
```

**影响**：Nova 的前端是"事件流状态机"，一条 `tool_call_completed` 丢失就永久停在"进行中"；Codex 的 UI 是"Item 集合的渲染"，重放能自愈。

---

## 6. 并发与取消

### Nova

**[实读]** `crates/nova-core/src/agent/cancellation.rs` 的模型：

```rust
begin_turn(Option<&str>) -> bool          // 占用
finish_turn(Option<&str>)                 // 释放
get_token(Option<&str>) -> CancellationToken
request_cancel(Option<&str>) -> bool
```

全局 `HashMap<conversation_id, CancellationToken>`（**推断**，基于 `cancellation.rs` 的 `get_token` 签名与 `engine.rs:27` 用法；未逐行读该文件）。

取消传播：`stream_runner.rs:243` 的 `select!` 有一个分支是 `cancel_token.cancelled()`。

### Codex

**[实读]** `CancellationToken` 从 `run_turn` 参数一路传到 `run_sampling_request`（`:1891` 参数列表），并被 `unified_exec`、工具执行共享。

并发由 `thread_manager.rs`（3233 行）管理 Thread 生命周期。

### 差异

| | Nova | Codex |
|---|---|---|
| 取消粒度 | conversation（整轮） | turn + step（`StepContext` 可被 preempt） |
| 取消生效点 | 流式读取处 | 采样、工具、PTY 读写 |
| 排队 | 拒绝 | ThreadManager |

**[签名]** Nova 有 `StepContext` 的对等物吗？没有 —— `engine.rs` 的循环变量是局部变量，没有结构化的步上下文。

---

## 7. 失败处理汇总

| 失败类型 | Nova 处理 | Codex 处理 |
|---|---|---|
| 上下文超限 | 字符串匹配 → 压缩重试一次 | `ContextWindowExceeded` → 压缩策略 |
| 限流 | 无专门处理 | `RateLimitExceeded` → 退避 |
| 服务端过载 | RPC 层 `-32001` + 客户端退避（`nova-client`） | `ServerOverloaded` 枚举 |
| 沙箱拒绝 | `bash` 工具内 `request_sandbox_retry` | orchestrator 统一 + Guardian 复审 |
| 工具循环 | `ToolLoopBreaker`（3 警告 / 5 熔断） | 无对等物 |
| 流挂起 | 180 秒看门狗（`stream_runner.rs` 新增） | `stream_max_retries` |
| 事件丢失 | 无恢复接口 | Item 重放 |

**Nova 独有的 `ToolLoopBreaker` 是设计亮点**，[实读] 且有测试覆盖（`engine.rs:778-840`）。

---

## 8. 端到端时序对比

### Nova

```
前端 send_modern_agent_turn(conversationId, prompt)
  → RPC → dispatch.rs → spawn(send_modern_agent_turn)
      → begin_turn（拒绝 if busy）
      → AgentEngine::execute_turn
          → emit TurnStarted
          → append_event(UserMessage + TurnStart)
          → assemble(history)  ← 文本注入工作区快照
          → loop {
                mid-turn compact
                stream_runner::run_streaming  ← 字符串判错
                tool_call → enforce_tool_permission（工具内）
                          → sandbox.wrap（工具内）
                          → 执行
                          → append_event(ToolCallCompleted)
                          → emit ToolCallCompleted
            }
          → handle_turn_error（所有错误） | TurnEnd
      → finish_turn
  → 事件经 nova-client → daemon.rs → app.emit → 前端
```

### Codex

```
client → app-server → ClientRequest::TurnStart
  → ThreadManager 排队
  → run_turn(Session, TurnContext, input)
      → hooks + 输入落库
      → guardian 检查
      → pre-sampling compact
      → run_sampling_request
          → ContextManager.for_prompt(modalities, image_store)
          → executed_tool_calls.attach_to_prompt
          → build_prompt
          → loop { 模型流 + ResponsesStreamRetryState }
          → ToolOrchestrator.run（审批 → 沙箱 → 执行 → 拒绝重试）
      → 事件 → event_mapping → Item → 协议通知
  → 客户端按 Item id upsert
```

---

## 9. 结论矩阵

| 维度 | Nova 状态 | 与 Codex 的关系 | 严重度 |
|---|---|---|---|
| 执行模型 | 单函数 614 行，无 StepContext | 结构不同 | 中 |
| 上下文构建 | 文本注入 | 机制不同 | 中 |
| 模型交互 | 字符串判错 + 手工重试 | **基础不同** | **高** |
| 工具调度 | 各工具自管 | **缺编排层** | **高** |
| 状态与事件 | 有投影，无重放 RPC | **缺接口** | **高** |
| 并发与取消 | 会话级，拒绝并发 | 粒度粗 | 中 |
| 失败处理 | 单一出口 | **缺分类** | **高** |

**四项"高"，本质是同一件事的三个投影**：错误没有类型 → 控制流只能靠字符串 → 无法分类恢复 → 只能整体失败。

## 10. 待办与优先级

| 优先级 | 项 | 触及文件 | 预估 |
|---|---|---|---|
| P0 | 错误类型化（先 6 类：限流/超限/网络/鉴权/模型/工具） | `provider/*`、`engine.rs`、`nova-protocol` | 1–2 天 |
| P0 | 状态重放 RPC（把 `projection.rs` 的四个函数暴露出去） | `dispatch.rs`、`session/projection.rs`、前端 | 2–3 天 |
| P1 | `ToolOrchestrator`（泛型 run：审批+沙箱+重试） | 新增 `agent/tools/orchestrator.rs`，改造各工具 | 3–5 天 |
| P1 | StepContext（步级上下文，为取消/timing 铺路） | `engine.rs` 重构 | 3–5 天 |
| P2 | 上下文改为 `ContextManager` 式（模态过滤 + 序号） | `context/assembler.rs` | 3–5 天 |
| P2 | MCP OAuth + 连接池 | `capabilities/mcp/` | 3–5 天 |

**不建议追**：124 crate 拆分、`guardian`、`realtime_conversation`、`parallel`（前三项对面试讲述无增量，`parallel` 可在 orchestrator 之后自然获得）。

## 11. 未验证项（诚实标注）

- `cancellation.rs` 内部实现：**[推断]** 基于调用点反推，未逐行读
- Codex `Session` 与 `ThreadManager` 的具体关系：**[签名]** 只读了 `run_turn` 参数与 `thread_manager.rs` 行数
- Nova `execute_turn` 的 375–742 段（工具循环细节）：**[部分实读]**，`ToolLoopBreaker` 通过测试代码确认存在
- Codex `state/` 与 `rollout` 的协作方式：**[统计]** 只有目录
