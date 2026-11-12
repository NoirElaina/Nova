# Nova × Codex 逐功能差异分析（代码实读版）

> ## ⚠️ 校准记录（2026-11-11）
>
> 本文写作于 unified_exec 替换**之前**，以下结论已过时，请勿再据此判断现状：
>
> | 过时位置 | 旧结论 | 现状 |
> |---|---|---|
> | §7 ⑧ / D-11（`:340-346`、`:874`） | 「Nova 一次性命令 + 后台作业 id，无 PTY 会话」 | 已实现 `exec_command` / `write_stdin` + `portable-pty`，见 `crates/nova-core/src/services/unified_exec/` |
> | §7 ⑨ / ⑩ | 存在 `BashKill` / `BashOutput` 工具 | 两者已删除；`list_background_jobs` / `kill_background_job` 只是旧 RPC 名转发到 unified_exec |
> | `:233` | 「只有 apply_patch 与 unified_exec 进 orchestrator」 | 这是 Codex 侧的描述；Nova 侧 `ToolOrchestrator` 只负责批量调度与熔断，审批/沙箱仍在 `execute_tool_with_app` 内，见 `crates/nova-core/src/agent/tools/orchestrator.rs:117-188` |
> | `:329` | 「Nova `apply_patch` 是 JSON function」 | 仍成立，且已补：全 hunk 权限校验、工作区相对路径解析、越界拒绝（`apply_patch/tool.rs:84-135`、`apply_patch/apply.rs:106-152`） |
>
> 仍然成立且**未修复**的差异：MCP 无 OAuth / 无取消转发；Windows 沙箱未接线（`sandbox/windows.rs` 仅保留实现）；协同看板此前为纯内存（现已补落盘，见 `orchestration/board.rs`）。
>
> **基准 A（参考）**：`D:/ref/codex/codex-rs`，commit `748ab55ca7de0b647d9a7a9d9a8e70e00f28666d`，2026-11-08，124 个 workspace crate / 5927 个 `.rs` 文件。
> **基准 B（本项目）**：`D:/Issue/MicrosoftVSCode/tauri/Nova`，5 个 workspace crate / 226 个 `.rs` 文件 / 11985 行 Rust，前端 123 个 `.vue` + 66 个 `.ts` / 9165 行。
> **路径约定**：下文 `C:` 前缀 = `D:/ref/codex/codex-rs/`；`N:` 前缀 = `D:/Issue/MicrosoftVSCode/tauri/Nova/`。所有行号均为本次实读时该文件的行号。
> **证据强度**：`[实]` 读过函数体并附摘录 · `[签]` 只读到定义/签名/注册点 · `[数]` 规模统计 · `[推]` 由调用点反推，未展开实现。凡 `[签]/[数]/[推]` 均在正文标注，不作为行为断言。
> **本文与 `docs/agent-architecture-audit.md` 的关系**：那版是七维框架式概览，多处为 `[签名]/[推断]/[统计]`；本文按**功能面逐项**重做，并在 §13 列出对上一版的 8 处修正。

---

## 0. 比对范围：为什么是这些维度

Agent 架构不是"有多少工具"，而是**一条输入到落库的链路上每一段是否有独立的可判定实现**。先把链路拆开，才知道该比什么：

```
输入提交 → 排队/并发 → 上下文构建 → 模型请求(wire) → 流式解析 → 工具调度
        → 审批 → 沙箱 → 工具执行 → 结果回填 → 事件落库 → 客户端协议 → UI 收敛
```

由此确定 8 个比对域、38 个子项。下表即本文的目录，也是"需要分析什么"的答案：

| # | 域 | 子项 | 本文位置 |
|---|---|---|---|
| 1 | 整体架构 | 进程模型、crate 分层、端到端数据流、配置与部署 | §2 |
| 2 | Agent 内核 | turn 执行模型、三层上下文对象、上下文构建、压缩、并发/取消、steer、状态与事件溯源、可观测 | §3 |
| 3 | 工具系统 | 定义/注册表/wire 形态、逐工具实现、统一执行管线、审批、沙箱、并行、渐进披露、熔断 | §4 |
| 4 | MCP | 传输、握手与协议版本、连接生命周期、工具发现与暴露、调用链、资源、审批、鉴权、elicitation、超时/重连/取消 | §5 |
| 5 | 协议 | LLM wire API、tool-calling 编码、SSE 事件与增量聚合、重试状态机、客户端 RPC、事件/Item 模型、TS 绑定与契约校验、传输帧 | §6 |
| 6 | 接口定义 | RPC 方法集、Tauri command、事件名、TS 类型、schema 生成 | §7 |
| 7 | 依赖配置 | workspace 组织、各 crate 依赖、前端依赖、能力映射 | §8 |
| 8 | 错误处理与测试 | 错误类型体系、错误传播、测试数量/形态/覆盖盲区 | §9–§10 |

**不在比对范围**（规模差异过大且对本项目行为无直接影响）：codex 的 guardian/attestation、realtime_webrtc、cloud-tasks、voice-host、code-mode 运行时、插件市场、企业合规（FedRAMP/EMA 的服务端）、Bazel 构建。这些在 §12 明确列为"不建议追"。

---

## 1. 结论速览

### 1.1 十条主要差异

| # | 差异 | 性质 | 严重度 |
|---|---|---|---|
| D-01 | **MCP 工具定义从未进入模型工具清单**：`collect_mcp_tools` / `connected_server_catalog` / `ToolRegistry::register_dynamic` 全仓无调用点；三个 provider adapter 只取内置 28 个工具 | 功能断链 | **P0** |
| D-02 | MCP 固定 `2024-11-05` 单版本、capabilities 发 `{}` 且丢弃响应；SSE transport 为 5 行 stub；无 OAuth/elicitation/连接池/取消/templates/订阅 | 协议代差 | **P0** |
| D-03 | 错误模型是 `Result<T,String>`（304 处）+ 字符串 `contains()` 分类；codex 是 `CodexErrorDetails` → `CodexErrorInfo`（22 变体）+ 类型化重试状态机 | 控制流基础 | **P0** |
| D-04 | 无流式重试状态机：provider 层零重试，只有 RPC 层 `-32001` 退避、压缩摘要重试、MCP 重连 | 可靠性 | **P0** |
| D-05 | 工具集是 28 个静态 `ToolRegistration`，无统一 `ToolSpec`；三种 LLM adapter 各写各的 tools 编码；codex 是 5 种 wire 形态 + 按 feature/model 动态注册 | 架构 | P1 |
| D-06 | 工具调度权限/沙箱分散在 3 层（`tools/mod.rs` 权限、`shell_sessions` 沙箱、`bash.rs` 提权重试），codex 用泛型 `ToolOrchestrator::run` 统一 | 架构 | P1 |
| D-07 | 客户端协议是严格 JSON-RPC 2.0 + 事件流；无 thread/turn/item 生命周期，无 Item id/status 的 upsert 语义（upsert 只在投影层按 `call_id` 配对） | 客户端健壮性 | P1 |
| D-08 | 契约校验只比 **方法名集合**（正则三方比对），无 JSON schema 生成、无产物快照测试；`bindings/protocol.ts` 是未跟踪的过期残留，与当前 Rust 类型冲突 | 契约漂移风险 | P1 |
| D-09 | 执行模型是单 614 行 `execute_turn` + `MAX_STEPS=30`；无 Session/TurnContext/StepContext、无 steer/排队、无步级 timing | 可扩展性 | P1 |
| D-10 | 测试：106 个内联单测 / 0 个 `tests/` 集成测试 / 0 个前端测试；codex 20129 个测试、62 个 `tests/` 目录、契约快照测试 | 质量保障 | P1 |

### 1.2 一句话总评

**Nova 在"单机会话型编码助手"这一范围内功能自洽且有几处优于 codex 的设计（`ToolLoopBreaker`、三协议适配、tree-sitter 命令静态分析、事件溯源投影）；差距集中在"边界"——MCP 生态接入、协议版本演进、错误/重试的类型化、客户端状态收敛。其中 D-01 是唯一一处"代码写好了但没接上"的硬缺口，其余多为深度差异。**

### 1.3 与 codex 的能力对照（粗粒度）

| 域 | Nova | codex | 判断 |
|---|---|---|---|
| 单轮 ReAct + 工具循环 | 有 | 有 | 等价，机制不同 |
| 三种 LLM 协议适配 | 有（3 种） | 仅 Responses API | **Nova 更宽** |
| 事件溯源 + 多投影 | 有（4 投影） | 有（thread history projection） | 等价 |
| 工具循环熔断 | 有 | 无 | **Nova 独有** |
| tree-sitter 命令静态分析 | 有 | 有（execpolicy 规则引擎） | 路线不同，Nova 更深于语法、codex 更广于策略 |
| 渐进披露 | `LoadTool` 精确名/全 token | `tool_search` BM25 + namespace 归一 | codex 更强 |
| MCP 生产化 | 弱（见 §5） | 强 | **主要差距** |
| 多智能体 | 单版本 + 进程内黑板 | v1/v2 + AgentControl + graph store + 持久化 | codex 更强 |
| 沙箱 | Linux bubblewrap 真实；Windows 实现未被选用 | 三平台 + 拒绝后升级重试 + Guardian 复审 | codex 更强 |
| hooks | 5 事件 | 14 事件 + 异步 drain | codex 更强 |

---

## 2. 整体架构

### 2.1 进程模型

**Nova**（`N:Cargo.toml:1-8`、`N:src-tauri/Cargo.toml`）：桌面壳与内核**分进程**。`src-tauri` 不依赖 `nova-core`，通过 `nova-client` 以 stdio JSONL 拉起 `nova serve` 子进程通信。

```
Vue3 (WebView) ──Tauri command──> src-tauri ──nova-client(stdio JSONL)──> nova-cli serve ──> nova-core
                                        ↑                                     │
                                        └────── event 通知 / app.emit ───────┘
```

证据：`N:crates/nova-client/src/lib.rs:73-90` 用 `Command::spawn` + `kill_on_drop(true)` + `Stdio::inherit()` 拉起内核；`N:src-tauri/src/daemon.rs:34-69` 是带退避（300ms→500ms×n→5s）的循环重连；`N:crates/nova-cli/src/serve/mod.rs:48-69` 是 `build_core` + `kernel.lock` 单实例 + 挂载 RemoteBrowser/RemoteHooks。

**codex**（`C:app-server/src/`、`C:app-server-transport/src/transport/mod.rs:83-89`）：单一 app-server 进程，传输可插拔 `Stdio / UnixSocket / WebSocket / Off`；exec-server 是另一进程；TUI/CLI/SDK 都作为客户端接入。

| 差异点 | 具体表现 | 产生原因 | 对项目行为的影响 | 证据 |
|---|---|---|---|---|
| D-2.1 内核独立进程 | Nova 内核是 daemon，桌面壳只是薄壳 | Tauri 侧避免链接重依赖、可复用 CLI/TUI/headless | 多一跳 IPC；daemon 崩溃要重连并重建订阅；内核可被 CLI 单独复用（**优点**） | `N:src-tauri/src/command/agent.rs:4-19`（薄壳）；`N:crates/nova-cli/src/serve/dispatch.rs:26-33` |
| D-2.2 单实例锁 | `kernel.lock` 阻止多个内核同时写同一 SQLite | 事件日志 append-only + `MAX(seq)+1` 需要写者唯一 | 多开 App 会失败；无法像 codex 那样多客户端共享一个 server | `N:crates/nova-cli/src/serve/mod.rs:76-110`；`N:crates/nova-core/src/agent/session/store.rs:38-75` |
| D-2.3 传输可插拔性 | Nova 固定 stdio JSONL（另有 WS 但仅 `/ws` Text 帧） | 桌面场景只要本机单客户端 | 无法远程接入、无法 UDS；codex 的 UDS 上甚至跑 WebSocket 升级 | `N:crates/nova-cli/src/serve/mod.rs:434-494,500-565`；`C:app-server-transport/src/transport/mod.rs:83-110`；`C:app-server-transport/src/transport/unix_socket.rs:193-245` |

### 2.2 crate 分层

| | Nova（5） | codex（124） |
|---|---|---|
| 纯类型层 | `nova-protocol`（含 ts-rs 导出） | `protocol`、`app-server-protocol`、`exec-server-protocol`、`code-mode-protocol`、`codex-backend-openapi-models` |
| 内核 | `nova-core`（单 crate，14107 行含 tools/mcp/provider/session/permissions） | `core` + 约 30 个功能 crate（`codex-mcp`、`rmcp-client`、`sandboxing`、`execpolicy`、`hooks`、`tools`、`rollout`、`state`、`thread-store`…） |
| 客户端 | `nova-client` | `app-server-client`、`app-server-test-client`、`codex-client`、`backend-client` |
| 入口 | `nova-cli`（serve/TUI/headless）、`src-tauri` | `cli`、`tui`、`app-server`、`app-server-daemon`、`exec-server` |

**影响**：Nova 单 crate 内核改动半径小、编译快，适合单人项目；codex 的拆分让 MCP/沙箱/协议可以独立版本化与测试（其 `rmcp-client/tests/` 有 37 个子模块专测 2026 协议发现），代价是 5927 文件的认知成本。**对面试讲述而言，Nova 的分层更清晰，不需要追 codex 的 crate 数。**

### 2.3 端到端数据流（实读）

**Nova**（`N:crates/nova-core/src/agent/engine.rs:128-740`）：

```
前端 send_modern_agent_turn
  → Tauri command → daemon::invoke → nova-client(JSON-RPC 2.0, id="rpc-N")
  → serve/dispatch.rs:26 → tokio::spawn → agent/mod.rs:18 send_modern_agent_turn
      → cancellation::begin_turn（占用会话，已占用则 Err）
      → AgentEngine::execute_turn
          → emit TurnStarted + StateChanged(AssemblingContext)
          → append_event(TurnStart / UserMessage)
          → load_events → projection::reconstruct_model_context → ContextAssembler::assemble
          → loop (MAX_STEPS=30):
              compact::compact_messages_mid_turn
              LlmClient::send_request → 三 adapter 之一 → SSE → Delta → AssistantOutputBuilder
              append_event(AssistantMessage / WireRequest / WireResponse)
              tool_calls 为空 → break
              append_event(ToolCall) + emit ToolCallRequested
              tools::execute_tool_calls_with_app
                  → 只读批 JoinSet(≤10) / 写工具串行
                  → pre-hook → jsonschema 校验 → enforce_tool_permission → 沙箱 → 执行 → post-hook
              ToolLoopBreaker 判定 → append_event(ToolResult) + emit ToolCallCompleted
              组装 ToolResult User 消息 → ContextMessage 落库 → push 上下文
          → append_event(TurnEnd) + emit TurnFinished
      → finish_turn
  → event_pump → {"jsonrpc":"2.0","method":"event",...} → app.emit → 前端 agent-listener
```

**codex**（`C:core/src/session/turn.rs:217-1012`）：

```
client → app-server(JSON-RPC dialect) → ClientRequest::TurnStart
  → ThreadManager 排队 / steer → run_turn(Session, TurnContext, input, cancel_token)
      → 音频模态校验、AutoCompactionDisabled、guardian pending input、drain_async_hook_results
      → pre-sampling compact（失败降级 Ok(None)）
      → capture_step_context_with_required_mcp_servers（生成 StepContext + tool_router）
      → run_hooks_and_record_inputs(PersistContext::TurnStart)
      → loop:
          InputQueue::get_pending_input（steer）
          clone_history → run_sampling_request
              → ContextManager.for_prompt(modalities, image_store)
              → executed_tool_calls.attach_to_prompt
              → build_prompt(model_visible_specs)
              → ResponsesStreamRetryState 重试循环（含 WS→HTTPS 回退）
          ToolOrchestrator.run（审批 → 沙箱 → 执行 → SandboxErr::Denied 升级重试）
          parallel: supports_parallel ? RwLock.read() : RwLock.write()
      → 事件 → event_mapping → ThreadItem(id/status) → item/started|delta|completed 通知
  → 客户端按 Item id upsert 收敛
```

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-2.4 事件落库时机 | Nova 每步 `append_event` 到 SQLite；codex 写 rollout + state DB | Nova 选择"事件即事实源" | Nova 前端丢事件可重拉历史（有 RPC），但无 Item 级实时幂等；codex 可按 Item id upsert 自愈 | `N:crates/nova-core/src/agent/session/store.rs:38-120`；`C:rollout/src/recorder.rs:1073-1097` |
| D-2.5 事件出站丢帧 | Nova `event_pump` 队列满时**主动丢一条并告警** | 有界出站队列 + 无背压协议 | 高速流式下 UI 可能缺一个 delta；无 `Last-Event-ID` 续传 | `N:crates/nova-cli/src/serve/mod.rs:248-272`；对比 `C:rmcp-client/src/http_client_adapter.rs:539`（MCP 侧有 last-event-id，但 app-server 侧靠 Item 重放） |
| D-2.6 模型请求构造位置 | Nova 在各 provider adapter 的 `prompt.rs`；codex 在 `core/src/client.rs:1295-1314` 单点 | Nova 三协议适配需要各写一份 | 加一个字段要改三处；codex 只改一处 | `N:crates/nova-core/src/provider/adapters/{openai,anthropic,responses}/prompt.rs`；`C:core/src/client.rs:1295-1314` |

---

## 3. Agent 内核（按可判定机制逐项）

### 3.1 turn 执行模型

| 项 | Nova | codex |
|---|---|---|
| 入口 | `agent/mod.rs:18-34` `send_modern_agent_turn` | `core/src/session/turn.rs:217-224` `run_turn` |
| 主体 | `engine.rs:128-740` 单函数，内含 30 次 ReAct 循环 | `turn.rs:217-1012` 拆成 7 段责任函数 + `try_run_sampling_request:2888-3827` |
| 上下文对象 | **无**。状态全是 `execute_turn` 局部变量（`current_messages`/`loop_count`/`total_turn_tokens`/`loop_breaker`） | `Session`（`session/session.rs:74-121`）/ `TurnContext`（`turn_context.rs:345-409`）/ `StepContext`（`step_context.rs:25-56`）三层 |
| 步级状态 | 无 | `StepContext{ sampled_through_sequence, tool_router, preempt, settings, environments, loaded_agents_md }` |
| 并发 | `begin_turn` 占用 `HashMap<conversation_id, CancellationToken>`，第二个 turn 直接 `Err("该会话已有正在进行的回复…")` | `ThreadManager` + `InputQueue` + `start_or_steer_turn`，支持同轮 steer |
| 循环上限 | `MAX_STEPS = 30`（`engine.rs:252`） | 由 follow-up / token 状态 / stop hooks 决定，无硬编码 30 |
| 取消 | 全局 token 穿透到循环顶、推理后、工具 `select!`、审批等待、子代理级联 | 同一 token 从 `run_turn` 传到 sampling、PTY 读写、`AbortOnDropHandle` |

**差异 D-3.1（执行模型）**
- **表现**：Nova 一轮的输入、历史、步数、token、熔断状态全在栈上；codex 把"线程 / 轮次 / 单次采样"做成三个对象，`StepContext` 里甚至固化了**这一轮确切的工具计划** `tool_router`。
- **原因**：Nova 是单会话桌面助手，一次只有一个 turn；codex 要支持多客户端、steer、远端环境、子代理、guardian 复审等跨 turn 状态。
- **影响**：① Nova 无法"取消后从某个 step 恢复"，只能整轮重来；② 无法按步统计（codex 的 `turn_timing` critical path 分段 inference/tools/auto_review/user_wait 依赖 StepContext）；③ 连点两次发送弹错误是并发模型的直接后果，不是 UI bug；④ 工具计划在 Nova 里是每轮 `get_available_tools_for_agent` 现算，codex 在 step 捕获时冻结，避免采样中途工具集变化。
- **证据**：`N:crates/nova-core/src/agent/mod.rs:18-34`、`N:crates/nova-core/src/agent/engine.rs:128-134,248-256,683-689`、`N:crates/nova-core/src/agent/cancellation.rs:25-45`；`C:core/src/session/turn.rs:217-224,341-364,616-658`、`C:core/src/session/step_context.rs:25-56`、`C:core/src/codex_thread.rs:549-583`。

### 3.2 上下文构建

**Nova**（`N:crates/nova-core/src/agent/context/assembler.rs:21-234`，5 步全实读）：
`drop_orphan_tool_blocks` → `strip_stale_workspace_contexts` → `build_workspace_snapshot`（探测技术栈 + 顶层目录骨架）→ `inject_fresh_workspace_context`（只注入最后一条 User 消息）→ `normalize_role_alternation`（合并相邻 User）。本质是**文本注入**。
系统提示词在 `N:crates/nova-core/src/agent/utils/system_prompt.rs:114-271` 由 8 段拼成（bundle 整体替换 / 占位符 / On-Demand Tools 目录 / Memory / Skills / Agent Resources / 模式约束），**由 provider adapter 调用，不在 engine 里**。

**codex**（`C:core/src/context_manager/`）：`history.rs` 的 `ResponseItemEnvelope` 带 `history_sequence` / `sampled_through_sequence` 单调序号；`normalize.rs:21-90` 补齐缺失的 `FunctionCallOutput`/`ToolSearchOutput`/`CustomToolCallOutput`；`history.rs:704-736` `for_prompt(input_modalities, image_store)` 按模型支持的模态剥离不支持的图片/音频，用 `AttachmentStore::is_resolvable` 并发 8 路替换不可解析图片；`updates.rs` 做增量世界状态更新；工具结果有独立通道 `executed_tool_calls.attach_to_prompt`（`C:core/src/tools/executed_tool_calls/request_metadata.rs:5-61`）。

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-3.2 工作区状态 | Nova 序列化成 `<workspace_context>` 文本塞进最新 User 消息，每轮先剥离旧的再注入新的 | 无结构化世界状态对象 | 模型看到的是"本轮重建的文字快照"；跨轮残留风险由剥离逻辑兜住，但无法做差量 | `N:.../context/assembler.rs:50-88,91-208` |
| D-3.3 模态过滤 | Nova 只在压缩摘要和 UI 投影里把 Image 换占位符；assemble 本身不过滤；`supports_image_input` 有导出但未见调用 | 三协议适配时按 adapter 处理 | 给不支持图片的模型发图片块会由上游报错，属**反应式**而非**预防式** | `N:.../utils/model_context.rs:1-7`；`N:.../capabilities/compact/summary.rs:9-44`；对比 `C:core/src/context_manager/history.rs:704-736` |
| D-3.4 序号与重放 | Nova DB 有 `seq`（`UNIQUE(conversation_id, seq)`），但模型消息无序号，重试时无法定位"从哪条重放" | 事件序号只用于落库排序 | 重试只能整轮重发（见 D-04） | `N:.../session/store.rs:21-34`；`C:core/src/context_manager/sampling.rs:1-76` |
| D-3.5 工具结果通道 | Nova 直接 `current_messages.push(Message{role:User, Content::Blocks(ToolResult)})`；codex 用 `attach_to_prompt` 带 `retry_cache` 与完整性归一 | codex 要支持增量工具（incremental_tools）与压缩时保留 | Nova 的 tool_use/tool_result 配对靠 `engine.rs:644` 的注释约束（"必须紧跟 ToolUse 的 Assistant 消息"），无类型级保证 | `N:.../engine.rs:630-659`；`C:core/src/tools/executed_tool_calls/request_metadata.rs:12-61`；`C:core/src/context_manager/normalize.rs:21-90` |

### 3.3 压缩

| 项 | Nova (`capabilities/compact/`) | codex (`core/src/compact*.rs`) |
|---|---|---|
| 触发 | 每次请求前 `compact_messages_mid_turn`（`engine.rs:296-302`）；阈值 50% context-edit / 80% Micro / 90% Full（`mod.rs:279-302`） | PreTurn / MidTurn / PostTurn / Guardian 溢出重试四类触发点（`turn.rs:745-749,869-891,917-946`） |
| 实现 | 本地 Micro 截断 + 本地 LLM 摘要（`summary.rs:154-183`）；token-budget 无 | 本地 Responses 摘要 / 远端 V2 / `compact_token_budget::start_new_context_window` 三路（`turn.rs:1710-1787`） |
| token 预算 | 按 `context_window_for_model` 的百分比 | `ContextWindowTokenStatus` 的 Total / BodyAfterPrefix 两种口径 + fallback buffer + 硬顶（`session/context_window.rs:7-118`） |
| 失败降级 | 熔断 `MAX_CONSECUTIVE_AUTO_COMPACT_FAILURES=3`（`state.rs`）；reactive 链 Micro→Full→保留 8 条→None（`mod.rs:684-728`） | `ContextWindowExceeded` 时逐条裁头部 + `backoff` + 换模型重试（`compact.rs:343-394`、`compact_model_fallback.rs:10-33`）；PostTurn 失败仅 warn 保留答案 |
| pending 保护 | `needs_user_input` 与最近一轮工具结果豁免截断（`mod.rs:400-405,431-482`） | `compact_pending_input.rs:10-49` 只保留未采样的 mail（≤10k tokens）+ `compact_retained_tool_outputs.rs` |
| 收敛 | `SessionEvent::CompactBoundary`，`reconstruct_model_context` 遇之丢弃之前全部（`projection.rs:451-467`） | 安装新历史 + `auto_compact_window` 推进 |

**值得注意的实读发现**：`compact_messages_for_turn` / `compact_messages_for_turn_with_report`（`N:.../compact/mod.rs:814,865`）**全仓无调用点**，是休眠代码；已接线的是 mid-turn 与 reactive。即"轮开始压缩"在 Nova 实际不生效。

### 3.4 工具调度管线（结构差异最大）

| 层 | Nova | codex |
|---|---|---|
| 定义 | `ToolRegistration{ tool: fn()->Tool, execute_with_app, permission, read_only, disclosure }`（`N:tools/mod.rs:209-237`）静态表 28 项 | `ToolSpec` 5 种 wire 形态 + `CoreToolRuntime: ToolExecutor<ToolInvocation>` trait（`C:core/src/tools/registry.rs:64-130`） |
| 注册 | `builtin_tool_registrations()` 一次构造 `OnceLock<ToolRegistry>`（`N:tools/mod.rs:35-64`、`registry.rs:103-120`） | `spec_plan.rs:122 build_tool_router` 按 feature/model_info/policy **动态注册**，同一工具可 Direct/Deferred/CodeModeOnly/Hidden |
| 分发 | `execute_tool_calls_with_app` → `execute_single_tool_call` → `execute_tool_with_app`（`N:tools/mod.rs:669-698,416-554,822-881`） | `ToolRouter::build_tool_call`（`C:core/src/tools/router.rs:278-347`）→ `ToolRegistry::dispatch_any_with_state`（`registry.rs:729-1038`） |
| 审批 | `enforce_tool_permission` 在 `execute_tool_with_app` 里统一调一次（`N:tools/mod.rs:841-857`），但**沙箱在 `shell_sessions` 里**、**提权重试在 `bash.rs` 里** | 泛型 `ToolOrchestrator::run`（`C:core/src/tools/orchestrator.rs:125-646`）统一：审批 → 选沙箱 → 执行 → `SandboxErr::Denied` → 二次审批 → 无沙箱重试 |
| 并行 | 只读工具 `JoinSet` 攒批并发（≤10），遇写工具先 flush（`N:tools/mod.rs:556-698`） | `RwLock<()>`：`supports_parallel_tool_calls()` 为 true 拿读锁并行，false 拿写锁独占（`C:core/src/tools/parallel.rs:291-321`） |
| 工具结果 | `ToolOutcome{output:String, additional_messages, prevent_continuation, stop_reason}` | `Box<dyn ToolOutput>` + `to_response_item()` 多态 |
| 失败 | `ToolFailure{kind, message, ...}`，7 种 kind | `FunctionCallError::{RespondToModel, Fatal}` + `ToolError::{Rejected, Codex}` |

**差异 D-3.6（编排层）**
- **表现**：Nova 的"审批 → 沙箱 → 执行"被拆到三个不同 crate 位置：`permissions/mod.rs:522`（审批决策）、`services/shell_sessions/mod.rs:305-333`（`wrap_shell` 包装 shell 进程）、`tools/bash_tool/bash.rs:146-178`（沙箱内失败 → `request_sandbox_retry` → `run_once_unsandboxed`）。只有 Bash 有拒绝后重试，其余 27 个工具没有。
- **原因**：Nova 的沙箱是"包 shell 进程"而不是"包工具执行"，所以沙箱天然只覆盖 shell 类工具；codex 把沙箱抽象成 `ToolRuntime::run(req, &SandboxAttempt, ...)`，apply_patch 与 exec 都实现同一个 `Sandboxable`/`Approvable` trait。
- **影响**：① 新增一个需要沙箱的工具，Nova 要在工具内部重写审批+沙箱+重试，codex 只需实现 `ToolRuntime`；② Nova 的提权重试顺序是"先沙箱内执行失败 → 再问用户 → 再沙箱外重试"，codex 是"先审批 → 选沙箱 → 执行 → 拒绝后二次审批 + Guardian 复审"，安全语义不同（Nova 让用户看到的是失败结果，codex 让用户看到的是明确的 escalation 请求）；③ Nova 的 `apply_patch` 完全不经沙箱（只走文件写权限描述符），`Edit`/`Write`/`MultiEdit` 同理。
- **证据**：`N:.../tools/mod.rs:822-881`、`N:.../permissions/mod.rs:522-621,628-672`、`N:.../sandbox/mod.rs:284-296`、`N:.../services/shell_sessions/mod.rs:305-333`、`N:.../tools/bash_tool/bash.rs:146-213`；`C:core/src/tools/orchestrator.rs:125-646`、`C:core/src/tools/runtimes/apply_patch.rs:120-262`、`C:core/src/tools/sandboxing.rs:196-410`、`C:sandboxing/src/manager.rs:275-356`。

> **修正上一版审计的一处说法**：`agent-architecture-audit.md:312` 写"所有工具（apply_patch、unified_exec、mcp…）走同一条路径"。实读显示 `ToolOrchestrator` 只被 `apply_patch`（`C:.../handlers/apply_patch.rs:606-611`）与 `unified_exec` 的 runtime 调用；约 30 个控制类工具（plan/view_image/tool_search/clock.*/request_user_input…）在 handler 内直接执行，不进 orchestrator。准确说法是：**"需要 OS 级副作用审批与沙箱的两个工具走统一编排，其余走统一的 registry dispatch（hooks/遥测/生命周期）"**。

### 3.5 权限系统

**Nova**（`N:.../agent/permissions/`，含 `bash_ast/`）：三层——描述符（`RiskLevel{Safe,Risky,Forbidden}`）→ 持久化规则（`permission_rules.json`，命令前缀匹配、deny 优先）→ `ApprovalPolicy{AlwaysAsk,OnRequest,Never}` → 会话允许集 → AskUser。`bash_ast/parser.rs` 用 tree-sitter-bash 解析到 `SimpleCommand{argv,env_vars,redirects,text}`，9 种 `RedirectOp`，对命令替换/进程替换/子 shell/控制流/展开一律 `TooComplex → Risky`（fail-closed）；`semantics.rs` 拦 eval-like builtins；`wrappers.rs` 有约 60 个只读命令 allowlist + git 子命令级检查，明确把 `python/node/sed/awk/find` 排除。

**codex**（`C:execpolicy/` + `C:protocol/src/permissions/`）：`Decision{Allow,Prompt,PromptUser,Forbidden}` 规则引擎，`Policy` 有 `insensitive_rules`/`rules_by_program`/`network_rules`/`host_executables_by_name` 四类索引；`core/src/command_canonicalization.rs:14-38` 把 shell 脚本规范化成 `__codex_shell_script__` 前缀以复用审批缓存；审批决策链是 hook → guardian → user（`C:core/src/tools/approvals.rs:561-703`），managed human checkpoint 不能被 hook 或自动审查替代。

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-3.7 审批决策者 | Nova 只有"规则 / 会话允许集 / 用户"三者；codex 多 guardian 自动审查与 hook 投票，且区分 `Prompt` vs `PromptUser`（后者强制真人） | Nova 无 review 模型 | Nova 的 `OnRequest` 策略下所有 Risky 操作都弹窗，无自动放行；codex 可在 guardian 结论下免打扰 | `N:.../permissions/mod.rs:522-621`；`C:execpolicy/src/decision.rs:9-19`；`C:core/src/tools/approvals.rs:600-683` |
| D-3.8 命令解析深度 | Nova tree-sitter AST 到参数/重定向/管道/展开；codex 是规则匹配 + 规范化 | 路线选择不同 | **这是 Nova 局部优于 codex 的地方**：对 `$(...)`、`bash -c`、`> $VAR` 等动态展开能静态判 Risky | `N:.../permissions/bash_ast/parser.rs:133-164,272-325,385-389`；`N:.../permissions/checks.rs:105-218` |
| D-3.9 MCP 操作评估 | Nova `assess_mcp_operation` 按 server/tool 名模糊匹配 shell/文件类，复用 `check_command` | MCP 工具无注解可用 | 无 `readOnlyHint`/注解驱动的审批，风险判断靠名字启发式 | `N:.../permissions/checks.rs:347-400`；对比 `C:core/src/mcp_tool_call.rs` 用 annotations + 审批模板（`C:.../mcp_tool_approval_templates.rs:53-124`） |
| D-3.10 子代理审批 | 子代理无审批通道，直接 Deny（`permissions/mod.rs:599-604`） | 白名单已限制为只读 | 子代理无法请求任何提权，安全但能力受限 | `N:.../permissions/mod.rs:599-604` |

### 3.6 沙箱

**Nova**：`SandboxPolicy{ReadOnly,WorkspaceWrite,DangerFullAccess}`，默认 **`DangerFullAccess`**（`N:.../sandbox/mod.rs:24-37`）；Linux 走 `BubblewrapSandbox`（`--ro-bind / /`、`--unshare-user/ipc/pid/net`、`--cap-drop ALL`），非 Linux 走 `NoSandbox`，且 `NoSandbox` 在 enforcing 策略下直接返回 Err（fail-closed）。`sandbox/windows.rs` 有 `WRITE_RESTRICTED` 受限令牌 + 合成 SID + ACL 的完整实现，但 `active_driver()` 在 `#[cfg(target_os="windows")]` 分支**仍返回 `&NoSandbox`**（`sandbox/mod.rs:277-280`）——即 Windows 上这份实现未被选用。

**codex**：`SandboxType{None,MacosSeatbelt,LinuxSeccomp,WindowsRestrictedToken,WindowsMxc}`（`C:protocol/src/sandbox.rs:29-61`），`get_platform_sandbox` 按平台选；`SandboxablePreference{Auto,Require,Forbid}`；拒绝后 `orchestrator.rs:436-613` 二次审批 + `SandboxType::None` 升级重试；还有 `managed_network` 审批（`network_approval.rs`）与 Windows 读授权（`windows_sandbox_read_grants.rs:9-37`）。

**影响**：默认 `DangerFullAccess` + Windows 无驱动 = **Nova 在用户主力平台（Windows）上默认不隔离**，命令执行只靠权限描述符与审批；codex 在 Windows 有受限令牌。这是安全面向的差异，不是功能缺失，但面试若被问"沙箱怎么做的"要如实说明这层。

### 3.7 Hooks

| 项 | Nova | codex |
|---|---|---|
| 事件 | 5 个：`SubagentStart/Stop/PreToolUse/PostToolUse/PostToolUseFailure`（`N:.../hooks/types.rs:23-31`） | 14 个：+ `PermissionRequest/PreCompact/PostCompact/SessionStart/SessionEnd/UserPromptSubmit/Stop/Interrupt/PreModelRequest/PostModelRequest`（`C:protocol/src/protocol.rs:1806-1851`） |
| 配置 | `hooks.toml`，5 种 handler：`Command/Context/Block/StopWhen/StopOnError`（`N:.../hooks/config.rs:79-111`） | 发现引擎 + 插件/managed requirement + `HookHandlerType{Command,McpTool,Prompt,Agent}` |
| 异步 | `fire_and_forget=true` → `tokio::spawn` 后不等待、无回执、无 drain | `HookExecutionMode::Async` + `drain_async_hook_results(before_user_prompt)` 在 turn 开头与采样后落地（`C:core/src/hook_runtime.rs:819-858`） |
| 阻断/改写 | Pre 可 `override_error`；Post 可注入 `additional_messages`、改写为失败、`prevent_continuation` | 同样有 `PreToolUseError::{Blocked,Failed,Cancelled}` 与 `with_updated_hook_input` 改写 invocation，另有 MCP hook 执行器 |

### 3.8 子代理与编排

**Nova**（`N:.../agent/orchestration/`）：`subagent.rs` 复用 `AgentEngine::execute_turn`，派生 `{parent}:sub:{uuid}`，`SUBAGENT_ALLOWED_TOOLS` 8 个只读工具白名单（`:24-34`），`Semaphore(3)`，报告截断 16000 字符，结束后删事件并写 graph/黑板；`board.rs` 进程内 `RwLock<HashMap>` 黑板（默认 general/research/evidence）；`graph.rs` 进程内拓扑 + BFS；`branch.rs` 纯文本分支问答，**不落库、无工具、无 hooks**。递归防护靠白名单排除 `Task`/`spawn_agent`，**无显式深度计数**。

**codex**：multi-agents v1（Namespace `multi_agent_v1`: spawn_agent/send_input/resume_agent/wait_agent/close_agent）+ v2（Function: spawn_agent/send_message/followup_task/wait_agent/interrupt_agent/list_agents），`AgentControl` trait（`C:core/src/agent/api.rs:41-99`）+ `AgentRegistry` + `agent-graph-store`（持久化 parent/child 拓扑 trait）+ `agent-message-board-host` + `agent-roles` + fork（`ForkPersistence::{Copied,Referenced}`）。

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-3.11 子代理通信 | Nova 一次性 `ToolOutcome` 回传 + 进程内黑板；codex 有 mailbox、`watch_mailbox`、send_input/resume/interrupt、跨 turn 存活 | Nova 子代理用完即弃（`delete_events`） | Nova 无法"先派一个 agent，几分钟后追问它"；子代理历史不保留 | `N:.../orchestration/subagent.rs:168-221`；`C:core/src/agent/api.rs:41-99`；`C:core/src/agent/control.rs:136-165` |
| D-3.12 `spawn_agent` 参数 | `role`/`nickname` 读取后赋给 `_role`/`_nickname` **直接丢弃**，只用 `task` | 未实现角色系统 | 模型传 role 无任何效果，schema 却声明了它 | `N:.../tools/spawn_agent_tool/spawn_agent.rs:24-42,57-69` |
| D-3.13 持久化 | board/graph 都是进程内 HashMap，重启丢失 | 单机单进程假设 | 重启后无法恢复多智能体拓扑 | `N:.../orchestration/board.rs:84-95`、`graph.rs:36-47`；对比 `C:agent-graph-store/src/store.rs:13-48` |

### 3.9 状态与事件溯源

**Nova 是做得比较完整的一块**：SQLite 三表 `conversations` / `session_events`（append-only，`UNIQUE(conversation_id, seq)`）/ `token_usage_log`；`SessionEvent` 11 变体；**4 个投影函数**：`render_ui_history:176`、`render_tool_logs:388`、`reconstruct_model_context:451`、`render_turn_traces:488`（`N:.../session/projection.rs`）。engine 每轮都 `load_events → reconstruct_model_context` 重建上下文（`engine.rs:235-246`），`CompactBoundary` 是唯一收敛点。

**codex**：rollout recorder + state DB + thread-store，事件 → `ThreadItem`（`id` 在每个 payload 变体内，`status` 是各变体自己的字段）→ `item/started|delta|completed` 通知；`thread_history_projection.rs:17-99` 把 `RolloutLine` 投影成 `ThreadHistoryChangeSet`；fork 支持 `Copied`/`Referenced`（`C:core/src/thread_manager.rs:1717-1835`）。

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-3.14 实时幂等 | Nova 事件是"发生过什么"（`ToolCallRequested`/`ToolCallCompleted` 靠 `call_id` 配对），upsert 只在投影层做；codex 事件是"有哪些 Item"，同 id 可重复推送 | 协议模型不同 | 前端若丢一条 `ToolCallCompleted`，UI 会永久停在 running，直到整页重拉历史；codex 重放一条 `item/completed` 即自愈 | `N:crates/nova-protocol/src/agent.rs:7-74`；`N:.../session/projection.rs:388-446`；`C:app-server-protocol/src/protocol/v2/item.rs:276-341` |
| D-3.15 重放 RPC | Nova 有 `load_history` / `load_conversation_tool_logs` / `get_conversation_turn_traces` 三个投影 RPC，但无"从某序号增量拉取"接口 | 事件序号只用于 DB 排序 | 前端收敛只能全量重拉 | `N:.../api/history.rs:73-144`；`N:.../session/history.rs:746-776` |
| D-3.16 fork/分叉 | 无持久化 fork；`:branch:` 明确不落库 | 未实现 | 无法从历史中间分叉出新会话 | `N:.../orchestration/branch.rs:3-9`；`C:core/src/thread_manager.rs:1717-1835` |

### 3.10 取消、并发与可观测

- Nova 取消：`OnceLock<Mutex<HashMap<String, CancellationToken>>>`（`N:.../cancellation.rs:5-10`），`begin_turn` 占用、`request_cancel` 按会话或兜底全部、`get_token` 克隆。穿透到 `engine.rs:262,380,525`、`tools/mod.rs:467`、`permissions/mod.rs:491`、`subagent.rs:152-166`。**MCP 调用不接收该 token**（见 D-5.9）。
- Nova 并发：只读工具 `JoinSet` ≤10（`NOVA_MAX_TOOL_USE_CONCURRENCY`），子代理 `Semaphore(3)`；**无 turn 级队列/steer**。
- 可观测：tracing + `token_usage_log`（含 `cost_usd`）+ `ToolCallCompleted.duration_ms` + `WireRequest/WireResponse` 轨迹；**无 otel / metrics / turn 级 timing / trace-id**。codex 有 `TurnTimingState`（critical path 分段、TTFT/TTFM）、`direct-tool-timing`、otel exporter、Statsig metrics（`C:core/src/turn_timing.rs:27-91`）。

---

## 4. 工具系统：逐工具比对

### 4.1 工具体系的结构性差异

| 维度 | Nova | codex |
|---|---|---|
| 数量 | **28 个定义 / 26 个注册源**（`board_tool::registrations()` 一次返回 3 个） | 约 40+ 个内置工具，按 feature/model 动态裁剪；另有 MCP/dynamic/extension 动态注入 |
| 定义形态 | 单一 `Tool{name, description, input_schema}`（JSON Schema），全部 `type:"function"` 语义 | `ToolSpec` 5 种：`function` / `namespace` / `tool_search` / `web_search`（托管）/ `custom`（Freeform + lark grammar）（`C:tools/src/tool_spec.rs:20-56`，已实读验证） |
| 注册表 | `OnceLock<ToolRegistry>`，`HashMap` 小写键；`register_dynamic`/`unregister_dynamic` 标 `#[allow(dead_code)]` 且**无调用点** | `ToolRegistry{ IndexMap<ToolName, RegisteredTool> }` + `tool_policy` + `first_collision` 检测；`register_external_with_exposure` 由 MCP handler cache 调用（`C:core/src/mcp_tool_exposure.rs:235`，已实读验证） |
| 可见性 | `ToolDisclosure::{Core,Deferred}` + `LoadTool` 精确名/全 token 匹配（`N:.../load_tool_tool/load_tool.rs:52-62,93-160`）；子代理/分支/按 bundle 过滤 | `ToolExposure::{Direct,Deferred,CodeModeOnly,Hidden}` + `tool_search` BM25（`C:core/src/tools/handlers/tool_search.rs:334-348`）+ `normalize_search_spec` 归一为 namespace（`C:tools/src/tool_search.rs:98-132`） |
| schema 校验 | `jsonschema::validator_for` 在 `execute_single_tool_call` 里统一做；**找不到定义时直接 `Ok(())`**（`N:tools/mod.rs:266-296`） | `parse_arguments<T>` 在各 handler 内做，失败回 `RespondToModel`（`C:core/src/tools/handlers/mod.rs:90-97`） |
| 并行标记 | `read_only: bool`（15 个 true） | `supports_parallel_tool_calls()` 默认 false，逐个显式开启（`C:tools/src/tool_executor.rs:106-139`） |

### 4.2 逐工具对照（28 个，一个不漏）

> 每个工具给出：Nova 定义/执行位置、参数与关键行为；codex 对应物；差异的**表现 / 原因 / 影响 / 证据**。Nova 路径统一省略前缀 `N:crates/nova-core/src/agent/tools/`。

#### 4.2.1 文件读写与编辑族（7 个）

**① `Read`**（`read_tool/read.rs:76` 定义 / `:365` 执行 / `read_only=true, Core`）
- 参数：`file_path`(必), `limit`, `offset`(1-based), `pages`(PDF≤20)。支持 PDF（`lopdf`）、图片（真解码校验后 base64，`ToolOutcome::text(note).with_additional_messages(image_message)` `:448`）、文本（`MAX_TEXT_SIZE=2MB`、`{:>6}\t` 行号前缀）。权限描述符 `:26-74` 对 `.ssh/.aws/.gnupg/.git` 与 Unix 设备文件标 Risky，注释明确"只读决定是否需要审批，不阻止读取"。
- codex 对应物：无独立 `read` 工具，读文件走 `exec_command`；有 `view_image`（`C:core/src/tools/handlers/view_image.rs:95-207`），它按 `model_info.input_modalities.contains(Image)` **提前拒绝**（`:99-109`），输出 `FunctionCallOutputContentItem::InputImage`（`:238-245`），受 `FileSystemSandboxContext` 约束。
- **差异**：表现=Nova 一个工具承担文本/PDF/图片三模态且内联 base64；codex 拆成 shell 读 + 结构化 `view_image`。原因=Nova 面向 Claude 式工具集，codex 面向 Responses API 的 content items。影响=Nova 的图片块直接进 `Message.content`，三种 adapter 要各自转换（`openai/prompt.rs:190-196` 补占位文本、`responses/prompt.rs:115-128` 把含图片结果序列化成 JSON 字符串），而 codex 在 `for_prompt` 统一按模态过滤与替换。**Nova 缺少"模型不支持图片时预防式剥离"这一层**。
- 证据：`read_tool/read.rs:26-74,147-161,365-454`；`C:core/src/tools/handlers/view_image.rs:95-245`；`C:core/src/context_manager/history.rs:704-736`。

**② `Write`**（`write_tool/write.rs:14/:69`，`false, Core`，权限 `describe_file_write_permission`）：参数 `file_path`/`content`；`resolve_tool_path` 拒绝 UNC、相对路径、`..`（`utils/paths.rs:16-48`）；覆盖时只读 encoding 不还原行尾；`write_text_content_lf` 原子写。codex 无 `Write` 工具，写文件走 `apply_patch` Freeform 或 `exec_command`。**影响**：Nova 的整文件覆盖绕过了 patch 的 diff 追踪，codex 的 `turn_diff_tracker` 能基于 patch hunks 产出结构化 diff。

**③ `Edit`**（`edit_tool/edit.rs:12/:74`）：参数 `file_path/old_string/new_string/replace_all`；`shared/edit.rs:174-253` 精确匹配 + `find_trimmed_match_range` 行尾/两端 trim 容错 + `format_closest_match_diagnostic` 最近代码块诊断 + 原子写；`old_string` 含 `*** Begin Patch` 时自动路由 apply_patch 引擎（`:99-113`）。codex 对应 `apply_patch`（Freeform）。**差异**：Nova 用"字符串精确替换"，codex 用"带上下文的 hunk + lark grammar"；前者对重复代码块会拒绝（多处命中且 `!replace_all`），后者靠 `seek_sequence` 上下文定位。

**④ `MultiEdit`**（`multi_edit_tool/multi_edit.rs:13/:127`）：`edits[]` 顺序应用，任一失败整体不落盘（`shared/edit.rs:256-333`）。codex 无对应单工具（可在一次 apply_patch 里放多个 hunk）。**差异**：Nova 是"一个工具多次编辑"，codex 是"一个 patch 多个 hunk 一次原子应用"，语义等价但错误定位粒度不同。

**⑤ `apply_patch`**（`apply_patch/tool.rs:26/:105`）：参数 `patch:string`，description 规定 `*** Begin Patch/End Patch` 与 Add/Delete/Update/Move；子树 `parser.rs`（`StreamingPatchParser` 流式 `push_delta` 逐字符 + 宽容剥离 heredoc）、`seek_sequence.rs`（四级容错：精确→trim_end→trim→Unicode 印刷标点归一）、`text_file.rs`（CRLF 保留）、`apply.rs`（`PatchExecutionReport::to_json`）。权限 `:84-103` 解析首个 hunk 走文件写描述符，解析失败退化为 `signature="apply_patch"` + `Safe`。
- codex：`apply_patch_spec.rs:9-28` 是 **`ToolSpec::Freeform`**（lark grammar `assets/tools/apply_patch.lark`，已实读验证），**不是 JSON function**；`ApplyPatchToolType` 只有 `Freeform` 一个变体（`C:protocol/src/openai_models.rs:336-340`）；另有 `intercept_apply_patch`（`C:.../handlers/apply_patch.rs:423-449`）在 `exec_command` 收到 `apply_patch` CLI 时转交同一执行逻辑；执行走 `ToolOrchestrator` + `ApplyPatchRuntime`（`C:.../runtimes/apply_patch.rs:120-262`）。
- **差异**：表现=Nova 把 patch 文本塞进 JSON 字符串参数；codex 用自由格式工具（模型不包 JSON）。原因=Responses API `custom` 工具支持 grammar 约束解码，能显著降低语法错误。影响=① Nova 模型可能产出 JSON 转义错误的 patch；② Nova 的 `apply_patch` **完全不经沙箱**（`tool.rs:116` 直接 `std::env::current_dir()` + `execute_patch_hunks`），codex 的走 `Sandboxable`/`Approvable` + 拒绝后升级重试；③ Nova 的 `resolve_path` 忽略传入 `workspace_root`（`apply_patch/apply.rs:106-109`），实际由 `resolve_tool_path` 决定。
- 证据：`apply_patch/tool.rs:16-24,105-133`、`apply_patch/apply.rs:34-109`；`C:core/src/tools/handlers/apply_patch_spec.rs:9-28`、`C:core/src/tools/handlers/apply_patch.rs:287-337,423-449,606-611`、`C:core/src/tools/runtimes/apply_patch.rs:120-262`。

**⑥ `Glob`**（`glob_tool/glob.rs:35/:64`）：`pattern`/`path`；`glob::glob`，`SCAN_HARD_CAP=50_000`、`MAX_RESULTS=1_000`，按 mtime 倒序，排除 `.git/.svn`，Windows verbatim 前缀归一；权限对敏感路径标 Risky。codex 无独立 glob 工具（走 `exec_command` + `rg --files`）。**差异**：Nova 是原生 Rust 实现（`glob` crate），无子进程开销；codex 统一走 shell，受沙箱与审批覆盖。

**⑦ `Grep`**（`grep_tool/grep.rs:38/:202`）：`pattern/path/glob/output_mode{content,files_with_matches,count}/-A/-B/-C/-i/-n/multiline/head_limit/offset`；`find_rg_path`（`NOVA_RG_PATH` → resource_dir/bin/rg → `rg`），`tokio::process::Command`，Windows `CREATE_NO_WINDOW`，`MAX_OUTPUT_BYTES=512KB`；退出码 2 → `normalize_rg_error`。codex 无独立 grep 工具（走 `exec_command`）。**差异**：Nova 把 ripgrep 封装成结构化工具并自带输出截断，模型不用拼命令行；codex 依赖模型写 `rg` 命令，但因此统一受 execpolicy 与沙箱治理。

#### 4.2.2 Shell 族（3 个）

**⑧ `Bash`**（`bash_tool/bash.rs:22/:90`，`false, Core`，权限 `describe_shell_command_permission`）：参数 `command`(必)/`description`/`timeout`(默认 120000，最大 1800000)/`run_in_background`/`max_runtime_ms`；`try_intercept_apply_patch`（`:124,215-263`）在进程内解析 `*** Begin Patch` 直接执行（**绕过 shell**）；`MAX_OUTPUT_CHARS=30_000` 保头 2/3 + 尾 1/3；沙箱内失败 → `should_retry_outside_sandbox`（`:203-213`）→ `request_sandbox_retry` → `run_once_unsandboxed` → 输出注入 `sandboxRetried:true`；4 个内联测试（`:325-424`）。
- codex 对应 `exec_command`/`write_stdin`（`C:.../handlers/shell_spec.rs:22-147`、`unified_exec/exec_command.rs:134-277`）：`cmd/workdir/tty/yield_time_ms/max_output_tokens/shell/login/environment_id`；`MIN/MAX_YIELD_TIME_MS=250/30000`、`DEFAULT_MAX_OUTPUT_TOKENS=10_000`、`UNIFIED_EXEC_OUTPUT_MAX_BYTES=1MiB`、`MAX_UNIFIED_EXEC_PROCESSES=64`（`C:core/src/unified_exec/mod.rs:68-77`）；PTY 会话可 `write_stdin` 续交互；`SandboxDenied` 返回结构化 `ExecCommandToolOutput`（`:241-267`）；错误截断 900 字节（`:48`）。
- **差异**：表现=Nova 一次性命令 + 后台作业 id；codex 是常驻 PTY 会话（`session_id` + `write_stdin`），可交互、可 `yield_time_ms` 分片取输出。原因=codex 要支持 REPL/长时进程/交互式程序。影响=Nova 无法让模型"启动一个 dev server 然后往 stdin 输命令"，只能起后台作业再用 `BashOutput` 读日志；后台作业的输出上限（每流 512KB、200 行/12000 字符）也只是工具 description 里的声明（`bash_output.rs:21-23`），实现未实读。
- 证据：`bash_tool/bash.rs:22-79,90-195,203-213,265-323`；`C:core/src/tools/handlers/shell_spec.rs:22-147`、`C:core/src/tools/handlers/unified_exec/exec_command.rs:134-277`、`C:core/src/unified_exec/mod.rs:68-77`、`C:core/src/unified_exec/process_manager.rs:518-683`。

**⑨ `BashKill`**（`bash_kill_tool/bash_kill.rs:26/:56`）：`id` → `kill_background_job`；权限 `signature=bashkill:{id}`、`Risky`、warning "该作业及其子进程会被强制结束"。codex 对应 `write_stdin` 的 `terminate` 或 `terminate_process`（`C:.../process_manager.rs:1856`）。**差异**：codex 的进程管理有 64 个进程上限、超时 `terminate_confirmed`、进程组清理；Nova 只 `child.kill()`（MCP 侧 `:172-174`，shell 侧实现未实读）。

**⑩ `BashOutput`**（`bash_output_tool/bash_output.rs:13/:49`，`read_only=true`）：无 `id` 时列出全部后台作业，有 `id` 时 `read_background_output(id,None,tail_lines)`，`tail_lines` 默认 200/最大 5000。codex 对应 `write_stdin`（`chars=""` 时读输出）。**差异**：codex 的读输出与写输入是同一 PTY 通道并带 `yield_time_ms`，Nova 是日志文件轮询式。

#### 4.2.3 网络族（2 个）

**⑪ `WebFetch`**（`web_fetch_tool/web_fetch.rs:15/:104`，`read_only=true, Deferred`）：`url`/`prompt`；显式拒绝非 http(s) 并把 http 升级 https；`FETCH_CACHE`（TTL 900s / 64 条）；`send_with_retry`（3 次、250ms 基、仅 connect/timeout/request）；自定义重定向策略（跨 host 或 >10 跳 stop）；20s timeout；UA `Nova/0.1 WebFetch`；`MAX_BODY_BYTES=512KB`；`html_to_markdown`（`:282-388`）；2 个测试。
- codex：无本地 WebFetch。对应能力是 **Responses API 托管 `web_search`**（`C:core/src/tools/hosted_spec.rs:14-45`，已实读验证，`ToolSpec::WebSearch{external_web_access,indexed_web_access,filters,user_location,search_context_size,search_content_types}`）或扩展工具 `web.run`（`C:core/src/tools/spec_plan.rs:1705-1726`）。`C:core/src/web_search.rs:1-30` 只是把 `WebSearchAction` 渲染成展示文本的辅助函数，**不是工具实现**。
- **差异**：表现=Nova 用 reqwest 自己抓网页转 Markdown；codex 把搜索/抓取交给服务端托管工具或扩展。原因=codex 走 OpenAI Responses 生态，网络访问受服务端策略与 guardian 治理。影响=Nova 的网络请求**不经沙箱/网络策略**（`WebFetch` 无权限描述符），且 SSRF 防护仅靠 scheme/host 校验与重定向限制；codex 有 `NetworkSandboxPolicy` 与 `network_approval`（`C:core/src/tools/network_approval.rs`）。
- 证据：`web_fetch_tool/web_fetch.rs:43-50,104-251,282-388`；`C:core/src/tools/hosted_spec.rs:14-45`、`C:core/src/tools/spec_plan.rs:724-753`、`C:core/src/web_search.rs:1-30`。

**⑫ `WebSearch`**（`web_search_tool/web_search.rs:13/:61`，`read_only=true, Deferred`）：`query`(minLength 2)/`allowed_domains`/`blocked_domains`；Bing 优先、失败或零结果回退 DuckDuckGo Lite；`parse_bing`（`b_algo` + `/ck/a` base64url 解包）、`parse_duckduckgo_lite`（`uddg` 解包）；域名过滤 + URL 去重 + `MAX_RESULTS=10`；`SNIPPET_MAX_CHARS=220`；7 个测试。
- **差异**：表现=Nova 是**HTML 爬虫解析**，codex 是**API 托管工具**（无本地解析代码）。原因=无服务端搜索 API 可用。影响=① 引擎 HTML 结构变更即失效（这是最脆弱的一处实现）；② 无 `search_context_size`/`user_location`/图片搜索等托管参数；③ 无网络审批。
- 证据：`web_search_tool/web_search.rs:61-134,181-193,200-337,422-505`；`C:core/src/tools/hosted_spec.rs:14-45`。

#### 4.2.4 用户交互、计划与配置（4 个）

**⑬ `ask_user_question`**（`ask_user_question_tool/ask_user_question.rs:14/:151`，`false, Core`）：`questions[]`（1–4 项，每项 `question/header/multi_select/options[2-4]{label,description,preview}`）、`context`、`allow_freeform`（默认 **true**）；返回 `{"type":"needs_user_input",...}`，由 `stream_runner.rs:856-865` 的 `is_needs_user_input_payload` 识别后 `stop_for_user_interaction=true`（`engine.rs:624-628`）。
- codex 对应 `request_user_input`（`C:.../handlers/request_user_input.rs:46-152`，`DirectModelOnly`，Plan 模式下 blocking，Guardian 下记录 `VerifiedAnswer`）与 `request_user_input_async`（`:89-150`，发 `TurnItem::AgentMessage{delivery:Async,questions}`，**不等待回答**，返回 `{"accepted":true}`）。
- **差异**：Nova 只有同步一种，且停止判定靠**返回 payload 的字符串匹配**（`type=="needs_user_input"`），不是类型化控制流；codex 有同步/异步两种并有 `AgentMessageDelivery` 类型。影响=Nova 若工具输出里恰好含该字符串会误触发停止；`permission_runtime.rs:6-15` 还有一份同逻辑的副本，两处需同步维护。

**⑭ `TodoWrite`**（`todo_write_tool/todo_write.rs:13/:113`，`false` 故意标 false 以避免进并发队列）：`todos[]{content,status∈pending|in_progress|completed,priority}`；最多一个 `in_progress`；`TodoRegistry`（进程内 `Mutex<HashMap>`）+ `NovaEvent::TodoUpdated`。codex 对应 `update_plan`（`C:.../handlers/plan.rs:66-99`，发 `EventMsg::PlanUpdate`，输出固定 `"Plan updated"`，Plan 模式下拒绝）。**差异**：Nova 的 todo 状态是进程内内存，重启丢失且不落 `session_events`；codex 的 plan 是事件，可投影进 thread history。

**⑮ `write_plan`**（`write_plan_tool/write_plan.rs:13/:36`）：`plan`/`summary` → `plan_files::save_conversation_plan` + `NovaEvent::PlanUpdated`。**实现缺陷**：`execute_with_app_boxed`（`:86-97`）把 `Err` 也包成 `Ok(ToolOutcome::text({"type":"error",...}))`，即 **`is_error` 恒为 false**，模型会把失败当成功（上一版审计未发现）。另外 `N:crates/nova-protocol/src/misc.rs:80` 的 `ALWAYS_ON_TOOLS = ["ask_user_question", "WritePlan"]` 与注册名 `"write_plan"` 大小写不一致，且 `is_tool_enabled` 第 92/95 行 `contains(&tool_name)` 未归一化（第 101 行才 `normalize_name`）——**挂载 bundle 且 `enabled_tools` 不含它时，`write_plan` 会被过滤掉**。这两点已从实际代码验证。

**⑯ `config_tool`**（`config_tool/config.rs:18/:56`，`true, Deferred`）：唯一 action `get_runtime_config_summary`；输出 provider（`hasApiKey` 布尔、`baseUrl` 只留 scheme://host[:port]）、`skills.disabledCount`、`hooks.handlerCount`、`ui`，末尾 `"redacted": true`。codex 无对应工具（配置通过 app-server `config/*` RPC）。**差异**：Nova 让模型自查运行配置，codex 把配置作为客户端协议能力，模型不可见。

#### 4.2.5 子代理与编排族（5 个）

**⑰ `Task`**（`task_tool/task.rs:14/:61`，`read_only=true` 故意让多个 Task 可并行）：`task`/`description`；`run_subagent_start_hooks` → `subagent::run`（`Semaphore(3)`）→ `run_subagent_stop_hooks`；**唯一显式设置 `prevent_continuation`/`stop_reason` 的工具**（`:113-129`），且能把 hook 的 override_error 转成 `ToolFailure::hook`。codex 对应 v2 `spawn_agent`/`wait_agent`（`C:.../multi_agents_v2/`）。**差异**：Nova 的 Task 是**阻塞式**（等子代理跑完才返回），codex 的 `spawn_agent` 返回 `agent_id/nickname` 后可 `wait_agent`/`send_input`/`interrupt_agent` 异步协作。

**⑱ `spawn_agent`**（`spawn_agent_tool/spawn_agent.rs:19/:45`）：见 D-3.12，`role`/`nickname` 被丢弃。

**⑲–㉑ `board_post` / `board_read` / `board_list_channels`**（`board_tool/board.rs:36,62,86` / `:97,149,172`）：一次 `registrations()` 返回 3 个；post 把 `parent_conversation_id` 归到根会话、判定子代理身份、`global_message_board().post_message` + `NovaEvent::BoardPostCreated`；read 默认 `limit=20`。codex 对应 `agent-message-board-host`（`C:agent-message-board-host/src/lib.rs:1-6,44-48`）与 v2 `send_message`。**差异**：Nova 是进程内黑板，codex 有 host + client + 持久化 graph store；Nova 的 board 数据在子代理白名单内（8 个工具里 3 个是 board），是子代理间唯一通信通道。

#### 4.2.6 能力与披露族（3 个）

**㉒ `LoadTool`**（`load_tool_tool/load_tool.rs:15/:64`，`read_only=true, Core`）：`names[]` 或 `query` 二选一；精确名大小写不敏感 + `query_matches` 全 token 命中；`mark_disclosed`（会话级内存 + 磁盘 `data_dir()/disclosed_tools/{scope}.json`）；回传完整 `name/description/input_schema`，附 note "Loaded tools are callable starting from your next step"；2 个测试。
- codex 对应 `tool_search`（`C:.../handlers/tool_search.rs:187-348`）：`ToolSpec::ToolSearch{execution:"client"}`，BM25 `bm25::Embedder` + `Scorer`，`TOOL_SEARCH_DEFAULT_LIMIT=8`，`normalize_search_spec` 把 Function/Freeform/Namespace **统一归一为 `LoadableToolSpec::Namespace`** 并强制 `defer_loading=Some(true)`、清除 `output_schema`，输出 `ResponseInputItem::ToolSearchOutput{status:"completed",execution:"client",tools:[...]}`，与 Responses API 原生 tool_search 协议对齐。
- **差异**：表现=Nova 是"普通 function 工具 + 目录文本注入 prompt + 会话状态"；codex 是"协议级工具类型 + 检索引擎 + 结构化输出"。原因=Nova 要兼容三种 LLM 协议，无法用 Responses 专属的 `tool_search` wire 形态。影响=Nova 的检索是精确名/全 token 字符串匹配，无法按语义召回；且加载状态需自己持久化，多端不共享。
- 证据：`load_tool_tool/load_tool.rs:52-62,93-160`；`N:.../utils/system_prompt.rs:65-112`；`C:core/src/tools/handlers/tool_search.rs:209-277,334-348`；`C:tools/src/tool_search.rs:98-132`；`C:tools/src/tool_search.rs`（`LoadableToolSpec` 在 `C:tools/src/responses_api.rs:50-56`）。

**㉓ `Skill`**（`skill_tool/skill.rs:28/:146`）：`action{list,run}`/`skill`/`args`；按 `active_bundle` 决定加载来源；`run` 时 `truncate_chars(&skill.content, 20_000)` 把技能正文直接塞进工具结果。codex 的 skills 由 `core/src/skills.rs` + `build_skills_and_plugins`（`turn.rs:1260`）在 turn 层注入上下文，且有 `mcp_skill_dependencies.rs`。**差异**：Nova 是"模型主动调用工具取技能"，codex 是"turn 层被动注入"，前者省 token 但要求模型知道技能名（`system_prompt.rs:199-233` 有 `## Available Skills` 元数据兜底）。

**㉔ `memory`**（`remember_global_memory_tool/remember_global_memory.rs:25/:52`，工具名是 `memory`）：`action{add,replace,remove}`/`content`/`old_text`；写 `memory_dir`，返回 `{"ok":true,...,"note":"... do not repeat it."}`；是 `DEFAULT_ONLY_TOOLS`（专用 bundle 不可见，`N:crates/nova-protocol/src/misc.rs:82`）。codex 对应 `memories`/`ext/memories`/`core/src/config/memory*` 与 guardian retained context。**差异**：Nova 是文件级全局记忆 + prompt 注入（`system_prompt.rs:181-197`），codex 有版本化记忆与 citation（`C:protocol/src/memory_citation.rs`、`memory_version.rs`）。

#### 4.2.7 MCP 族（3 个）与 Browser（1 个）

**㉕ `list_mcp_resources`**（`list_mcp_resources_tool/list_mcp_resources.rs:23/:39`，`true, Deferred`）：`server` → `ensure_server_visible` → `mcp::list_mcp_resources` → `{"ok":true,"resources"}`。
**㉖ `read_mcp_resource`**（`read_mcp_resource_tool/read_mcp_resource.rs:23/:40`，`true, Deferred`）：`server`/`uri` → 同上 → `mcp::read_mcp_resource`。
**㉗ `mcp_auth`**（`mcp_auth_tool/mcp_auth.rs:24/:82`，`false, Deferred`）：7 个 action `status/reload_all/enable/disable/list_tools/probe_tool/call_tool`；`probe_tool|call_tool` 走 `call_mcp_tool_with_nested_permission`（`:213-220`）。**详见 §5，这是 D-01 的关键**。
- codex 对应 3 个 resource 工具（`list_mcp_resources`/`list_mcp_resource_templates`/`read_mcp_resource`，`C:.../mcp_resource_spec.rs:25-97`）+ MCP 工具直接暴露为 namespace（不需要网关工具）。**差异**：Nova 少一个 `list_mcp_resource_templates`；且把 MCP 管理/调用压进一个 `mcp_auth`，模型要先 `LoadTool` 加载它才能发现 MCP。

**㉘ `Browser`**（`browser_tool/browser.rs:10/:72`，`true, Deferred`）：`action{navigate,snapshot,click,type,reset}`/`url`/`ref`/`selector`/`text`/`x`/`y`/`clear_data`；`core.browser()` 取 bridge，无 bridge 直接失败；`bridge.run_command` 由桌面宿主提供（`src-tauri/src/services/browser_sessions/`、`src/features/browser/`）。codex 的浏览器是 `computer_use_config` / `browser_use_config` 协议配置 + 扩展工具（`C:app-server-protocol/src/protocol/v2/{browser_use_config,computer_use_config}.rs`），主仓无等价的本地 bridge 工具。**差异**：Nova 把浏览器做成"内核工具 → 桌面宿主 RPC"的反向能力调用（`/capabilities/browser`，`serve/mod.rs:170-209`），这是 Nova 架构上比较有特点的一处，codex 走的是配置化 computer-use。

### 4.3 codex 有而 Nova 完全没有的工具族（按功能列举）

| 工具 / 族 | 作用 | 证据 |
|---|---|---|
| `view_image` | 结构化图片输入 + 模态预检 | `C:.../handlers/view_image.rs:95-245` |
| `update_plan` | 事件化计划 | `C:.../handlers/plan.rs:66-99` |
| `tool_search` | 协议级工具检索 | `C:.../handlers/tool_search.rs:187-348` |
| `request_permissions` | 模型主动申请权限提升（含 `entries`、environment_id） | `C:.../handlers/request_permissions.rs:40-155` |
| `request_user_input_async` / `send_message_to_user_async` | 异步不阻塞的用户消息 | `C:.../request_user_input_async.rs:79-150` |
| `new_context` / `get_context_remaining` | 上下文窗口管理 | `C:.../new_context_window.rs:38-68`、`get_context_remaining.rs:85-106` |
| `clock.curr_time` / `clock.sleep` | namespace 工具族 | `C:.../current_time.rs:61-128`、`sleep.rs:46-160` |
| `multi_agent_v1.*`（5）/ v2（6） | 完整多智能体生命周期 | `C:.../multi_agents_spec.rs:88-370` |
| `code_mode.exec` / `wait` | 代码模式（JS 运行时编排工具） | `C:core/src/tools/code_mode/execute_spec.rs:5-25` |
| `dynamic` / `extension_tools` | 客户端注入工具、插件工具 | `C:.../handlers/dynamic.rs:34-173`、`extension_tools.rs:30-104` |
| `wait_for_environment` / `test_sync_tool` | 远端环境就绪 / 测试同步屏障 | `C:.../wait_for_environment.rs:89-168`、`test_sync.rs:85-196` |
| `list_available_plugins_to_install` / `request_plugin_install` | 插件市场 | `C:.../list_available_plugins_to_install.rs:58-103` |
| hosted `web_search` | Responses API 托管搜索 | `C:core/src/tools/hosted_spec.rs:14-45` |

### 4.4 共用设施差异

| 设施 | Nova | codex |
|---|---|---|
| schema 校验 | 统一 `jsonschema::validator_for`（`tools/mod.rs:266-296`），**未注册工具直接放行** | 各 handler `parse_arguments<T>`（`handlers/mod.rs:90-97`） |
| 并行 | 只读批 `JoinSet` ≤10，写串行，结果 `BTreeMap` 保序（`tools/mod.rs:556-698`） | `RwLock` 读/写锁 + `ToolAdmission::{Parallel,Exclusive}` 计时（`parallel.rs:291-321`） |
| 熔断 | `ToolLoopBreaker`：同签名连续失败 ≥3 警告、≥5 熔断、同参 ≥6 警告（`engine.rs:21-85,578-596`），**codex 无对等物** | 无 |
| 工具生命周期 | pre/post hook + `ToolFailureKind` + `emit_backend_error` | `notify_tool_start/finish`、`ControlToolCallGuard`、`ToolDispatchTrace`、otel span、`call_trace` |
| 路径安全 | `resolve_tool_path` 拒 UNC/相对/`..`，逐级 canonicalize（`utils/paths.rs:16-85`） | `PathUri` + `FileSystemSandboxContext` + `PermissionPath` |
| 原子写 | `atomic_write::write_bytes` + 1000 条/25MB 内容缓存（`utils/file_io.rs:30-32,161-203`） | `codex_apply_patch` + `turn_diff_tracker` |

---

## 5. MCP 子系统（逐能力比对）

### 5.0 先给结论：Nova 的 MCP 有"实现"但没"打通"

实读后的调用链如下（这是本文最重要的发现）：

```
模型可见工具清单 = get_available_tools_for_agent()          [tools/mod.rs:738-790]
        ↓ 只调 get_available_tools() = global_tool_registry().list_all_tools()   [:708-710, registry.rs:172-180]
        ↓ 只含 builtin_definitions + dynamic_tools（后者从未被写入）
        ⇒ 28 个内置工具，其中 MCP 相关只有 3 个 Deferred 工具 + mcp_auth 网关

collect_mcp_tools()          [mcp_tools/mod.rs:78]   ── 全仓只有定义处，无调用点  ✗
connected_server_catalog()   [mcp_tools/mod.rs:46]   ── 仅被 collect_mcp_tools 调用  ✗
ToolRegistry::register_dynamic() [registry.rs:124]   ── #[allow(dead_code)]，无调用点  ✗

三个 provider adapter 的工具来源：
  openai/prompt.rs:242  → tools::get_available_tools_for_agent(...)
  responses/prompt.rs:251 → 同上
  anthropic/prompt.rs:376 → 同上
  ⇒ 没有任何一处把 MCP 工具 append 进去

但执行侧可以解析 mcp__ 名字：
  execute_tool_with_app → mcp_tools::execute_dynamic_with_app  [tools/mod.rs:868-872]
  validate_tool_input 对未注册工具返回 Ok(())                 [tools/mod.rs:266-296]
  ⇒ 模型若"硬猜"出 mcp__<server>__<tool>，能被调到，但没有任何 schema 告诉它名字与参数
```

**差异 D-01（MCP 工具定义未接入模型清单）**
- **具体表现**：`collect_mcp_tools` 已经写好完整实现（枚举已连接 server → `tools/list` → 生成 `Tool{name:"mcp__{server}__{tool}", description, input_schema}`，`N:.../mcp_tools/mod.rs:78-102`），注释甚至写着"这将使模型可调用 `mcp__<server>__<tool>`"，但**没有任何调用点**；`register_dynamic`/`unregister_dynamic` 标 `#[allow(dead_code)]` 且无调用点；`servers_visible_to_agent`（`mcp/mod.rs:225`）同样只有定义处。`get_available_tools_for_agent` 的注释写"MCP 工具不参与披露，全量保留"（`tools/mod.rs:734-737`），但其返回集合只来自内置注册表，`None => true` 分支（`:785-786`）只对已注册项生效。
- **产生原因**：推测是渐进披露重构后，MCP 工具从"`get_available_tools()` 里动态拼接"改为"注册到 `ToolRegistry.dynamic_tools`"时，注入环节漏接（本次未做 git 历史考证，标 `[推]`）。
- **对项目行为的影响**：① 模型**无法直接发现或调用**任何 MCP 工具；② 唯一可用路径是先 `LoadTool` 加载 Deferred 的 `mcp_auth`，再用 `list_tools` 查看某 server 的工具列表，再用 `call_tool`/`probe_tool` 经嵌套权限调用（`mcp_auth.rs:164-220`）——即"网关式"而非"原生工具式"；③ `mcp__*` 名字的执行通道与 `is_read_only_tool`（`tools/mod.rs:298-308` → `dynamic_tool_read_only` 恒返回 `None`）存在但不可达；④ `list_mcp_resources`/`read_mcp_resource` 也只是 Deferred 工具，需先加载。
- **证据**：`N:crates/nova-core/src/agent/capabilities/mcp_tools/mod.rs:46-102`（全文实读）、`N:.../agent/tools/registry.rs:122-141`、`N:.../agent/tools/mod.rs:708-710,734-790,868-872`、`N:.../provider/adapters/{openai/prompt.rs:242,anthropic/prompt.rs:376,responses/prompt.rs:251}`、`N:.../agent/tools/mcp_auth_tool/mcp_auth.rs:164-220`。上述 grep 已在本机重复执行验证。

### 5.1 配置与传输

| 项 | Nova | codex |
|---|---|---|
| 配置枚举 | 3 变体 `Stdio{command,args,env}` / `Sse{url,headers}` / `StreamableHttp{url,headers}`（`N:crates/nova-protocol/src/mcp.rs:25-53`） | 2 变体 `Stdio{command,args,env,env_vars,cwd}` / `StreamableHttp{url,bearer_token_env_var,http_headers,...}`（`C:config/src/mcp_types.rs:658-672`） |
| 运行时连接 | 仅 2 种 `ServerConnection::{Stdio, StreamableHttp}`（`N:.../mcp/mod.rs:25-28`） | `PendingTransport` 5 变体：InProcess / Stdio / StreamableHttp / WithOAuth / WithAccessTokenOnly（`C:rmcp-client/src/rmcp_client.rs:126-143`） |
| SSE | **stub**：`sse.rs` 全文 5 行，固定返回 `"SSE MCP runtime not implemented yet. Use stdio for now."`；`mod.rs:391-396` 对该分支固定置 Error | 无独立 legacy SSE transport；SSE 是 streamable HTTP 的响应体/GET 流，支持 `Mcp-Session-Id`、`Last-Event-ID` 断线续传、DELETE session（`C:rmcp-client/src/http_client_adapter.rs:65-70,371-432,511-590`） |
| stdio 帧 | 手写 NDJSON：`serde_json::to_vec + b'\n' + flush`（`stdio.rs:20-28`）；`read_line` 解析失败静默 continue（`:30-52`）；`u64` 递增 id；非匹配 id 消息**直接丢弃**（`:54-89`） | `bounded_stdio_transport` 帧界、`MAX_MCP_STDOUT_LINE_BYTES=8MiB` / `MAX_MCP_STDERR_LINE_BYTES=1MiB`（`C:.../executor_process_transport.rs:48-53`）、事件通知传输、executor 进程传输 |
| stderr | `Stdio::null()` 丢弃（`stdio.rs:195-210`），Windows `CREATE_NO_WINDOW`；`shutdown()` 只 `child.kill()`（`:172-174`） | 有上限的行读取 + 进程组清理 + 独立 `process_group_cleanup` 测试 |
| HTTP 会话 | 提取 `mcp-session-id` 回传（`streamable_http.rs:16-23,166-168`），拒绝用户手配该 header（`:326-330`）；响应体兼容单 JSON / 数组 / SSE（`:72-123`）；**`shutdown()` 为空实现**（`:313-315`），无 GET 长连接、无 DELETE、无 session 失效重建 | 完整 streamable HTTP 客户端 + GET SSE 流 + DELETE + `is_session_expired_404` / `reinitialize_after_session_expiry`（`C:rmcp-client/src/rmcp_client.rs:1889-1973`，函数体未实读） |

**差异 D-5.1（SSE 配置是假实现）**
- 表现：配置层允许 `type:"sse"`，但连接时固定返回错误，UI 会把该 server 标为 `Error`。
- 原因：预留配置未实现运行时。
- 影响：用户按配置文档填 SSE server 会得到"未实现"提示，且这个失败会走 `mark_server_runtime_error` → 重连 → 再失败（`mcp/mod.rs:811-826`），产生一次无意义的 30s 超时。
- 证据：`N:.../mcp/sse.rs:1-5`（全文）、`N:.../mcp/mod.rs:391-396`、`N:crates/nova-protocol/src/mcp.rs:36-40`。

**差异 D-5.2（stdio 请求循环丢弃所有非匹配 id 消息）**
- 表现：`send_request` 只认与自己 `id` 相等的响应，服务端通知与**服务端主动请求（elicitation / sampling / roots）全部丢弃**。
- 原因：同步 request/response 模型，无后台读任务与分派器。
- 影响：无法支持 elicitation、`notifications/tools/list_changed`、`notifications/cancelled`、`ping`；MCP server 若发通知，客户端会一直读到下一个匹配 id 才返回（超时 30s 或挂起）。
- 证据：`N:.../mcp/stdio.rs:54-89`、`streamable_http.rs:136-158`；对比 `C:rmcp-client/src/elicitation_client_service.rs:215-310`（5 类服务端请求分派）。

### 5.2 握手与协议版本

**Nova**（`stdio.rs:263-290`、`streamable_http.rs:359-387` 逐字同构，已实读）：

```rust
let init_result = timeout(MCP_CONNECT_TIMEOUT, conn.send_request(
    "initialize",
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": { "name": "nova", "version": "0.1.0" }
    }),
)).await;
match init_result { Ok(Ok(_)) => {}, ... }   // 返回值被丢弃
let _ = conn.send_notification("notifications/initialized", json!({})).await;
```

**codex**（`C:codex-mcp/src/rmcp_client.rs:1358-1389`，已实读）：
```rust
let mut capabilities = ClientCapabilities::default();
capabilities.elicitation = Some(client_elicitation_capability);
if client_mcp_extensions.contains(MCP_NETWORK_POLICY_META_CAPABILITY) {
    capabilities.experimental = Some(BTreeMap::from([(
        MCP_NETWORK_POLICY_META_CAPABILITY.to_string(), serde_json::Map::new(),)]));
}
...
InitializeRequestParams::new(capabilities,
    Implementation::new("codex-mcp-client", env!("CARGO_PKG_VERSION")).with_title("Codex"))
    .with_protocol_version(ProtocolVersion::V_2025_06_18)
```
并有 `McpProtocolMode{Legacy,V20260728,DiscoverLocalStdio}`（`C:rmcp-client/src/protocol_mode.rs:9-36`）、`server/discover` 与 late-reply/reoffer/legacy 回退（`stdio_handshake.rs:18-79`、`local_stdio_transport.rs:64-196`）、HTTP 侧 `legacy_discovery_fallback_response`（`http_client_adapter.rs:820-910`）、2026 测试服务器（`bin/test_mcp_2026_discovery_stdio_server.rs:9-66`）。

**差异 D-5.3（协议版本代差）**
- 表现：Nova 固定 2024-11-05、capabilities 发空、丢弃 initialize 响应；codex 支持 2025-06-18 与 2026-07-28 双版本并按 server opt-in 升级。
- 原因：Nova 手写最小 JSON-RPC 客户端，未引入 `rmcp` SDK；codex 有独立 `rmcp-client` crate 并跟进 MCP 规范演进。
- 影响：① 新版 MCP server 若要求 2025-06-18 会握手失败；② 不读 capabilities ⇒ 不知道 server 是否支持 resources/tools/elicitation，后续调用可能发服务端不支持的方法；③ 无法协商网络策略、沙箱状态、connection lease 等扩展能力（这些是 codex 通过 capabilities 投影的安全面）；④ 无 `server/discover` ⇒ 无法做现代协议发现。
- 证据：`N:.../mcp/stdio.rs:263-290,267-280`、`streamable_http.rs:359-387`；`C:codex-mcp/src/rmcp_client.rs:1358-1389`、`C:rmcp-client/src/protocol_mode.rs:9-53`、`C:rmcp-client/src/stdio_handshake.rs:18-79`。

### 5.3 连接生命周期

| 能力 | Nova | codex |
|---|---|---|
| 启动 | `ensure_runtime_loaded`（`mcp/mod.rs:481-540`）**顺序 for 循环**，逐 server `connect_server`，无 `join_all`/`spawn`/`buffer_unordered`（已 grep 验证） | eager 全并发 `JoinSet`、deferred 逐个 `spawn`，`McpStartupPolicy{Eager,DeferCachedStdio,DeferCachedServers}`（`C:codex-mcp/src/connection_manager.rs:1136-1165`、`runtime.rs:74-83`） |
| 必需服务器 | 无 `required` 概念 | `validate_required_servers`（`C:.../connection_manager/required.rs:17-63`），允许缓存 catalog 满足 |
| 状态 | 3 态 `Disconnected/Connected/Error`（`N:crates/nova-protocol/src/mcp.rs:6-23`） | 7 态 `NotStarted/Starting/Connected/AuthenticationRequired/Failed/Cancelled/Disabled`（`C:protocol/src/mcp.rs:17-25`）+ `McpStartupUpdate`/`McpStartupComplete` 事件（`C:.../startup.rs:41-143`） |
| 状态推送 | **无 MCP 事件变体**（`N:crates/nova-core/src/events.rs:16-77` 13 变体里没有），前端轮询 `get_mcp_server_statuses` | 事件流驱动 |
| 超时 | 统一 30s（`mcp/mod.rs:91`），施加于 initialize / tools/list / HTTP client | 启动 30s、工具 300s（可配）、握手超时、`LEASE_REQUEST_TIMEOUT=5s`（`C:.../rmcp_client.rs:122-126`） |
| 重连 | `reconnect_server`（`mcp/mod.rs:435-469`）+ 4 个操作入口"首次失败 → mark error → 重连 → 再试一次"（`:811-826,869-884,927-942,986-1001`）+ 后台 `reload_all_mcp_servers`（`mcp_tools/mod.rs:59-67`） | 后台 `reconnect_in_background` + 指数退避 1s→30s（`C:.../rmcp_client.rs:266-271`）+ 会话 404 重初始化 |
| 连接池 | 无；每 server 单 `ServerConnection`，全局 `Mutex<HashMap>` 串行 | 2026 stdio 按 `ConnectionKey` 共享 + lease（`C:codex-mcp/src/connection_pool.rs:39-44,258-415`），每 chat 的 catalog/elicitation 隔离 |
| 工具缓存 | 启动时 `tools/list` **只取 `tools.len()`** 存 `tool_count`（`mcp/mod.rs:359-370`），不缓存列表；每次 `list_mcp_tools` 都真发请求 | `ClientToolCatalog` 带 revision、通知刷新冷却 5s、失败保留旧 catalog（`C:codex-mcp/src/client_tool_catalog.rs:171-233`） |
| 分页 | 无 | 100 页 / 2048 工具（Apps 8192）/ 64KB cursor / 重复 cursor 检测（`C:codex-mcp/src/pagination.rs:9-80`） |
| 配置迁移 | **有**：兼容 Claude/Cline `mcpServers` 对象格式、agent 私有 `mcp.json` 迁入全局、冲突自动改名 `<name>-2`（`mcp/mod.rs:131-158,282-345`）、headers 逐条加密落盘（`:175-221`） | 由 catalog 与 `EnvironmentMcpPolicy` 治理（`C:codex-mcp/src/catalog.rs:407-475`） |

**差异 D-5.4（串行启动 + 无状态推送）**
- 表现：N 个 server 串行握手，每个最坏 30s；前端只能轮询。
- 原因：单一 `Mutex<HashMap>` 状态 + 同步连接模型。
- 影响：配置 5 个不可达 server 时，预热最坏 150s；`connected_server_catalog` 的注释（`mcp_tools/mod.rs:59-63`）明确提到"重连可能耗时 30s，若在此等待会导致用户消息卡住"，说明作者已意识到并改用后台 spawn，但后台 `reload_all_mcp_servers` 仍是串行。
- 证据：`N:.../mcp/mod.rs:481-540,692-733`、`N:.../mcp_tools/mod.rs:59-67`；`C:codex-mcp/src/connection_manager.rs:1136-1165`。

### 5.4 工具发现与暴露

| 项 | Nova | codex |
|---|---|---|
| 命名 | `mcp__{server}__{tool}`（`mcp_tools/mod.rs:6-8`），`splitn(2,"__")` 解析（`:11-20`） | 同前缀常量 `MCP_TOOL_NAME_PREFIX="mcp"` / `DELIMITER="__"`（`C:codex-mcp/src/mcp/mod.rs:68-89`），但生成的是 `ToolSpec::Namespace`（`C:.../handlers/mcp.rs:550-581`） |
| 冲突 | 无去重、无长度限制、无 hash 后缀 | 原始名去重 + namespace 冲突 SHA1 后缀 + 128 字节上限（`C:codex-mcp/src/tools.rs:114-223,235-243`）+ catalog tier 冲突（`catalog.rs:305-317`） |
| schema 转换 | 直接透传 `input_schema`，缺省补空 object（`mcp_tools/mod.rs:88-98`） | `mcp_tool_to_responses_api_tool` 带 `schema_max_bytes` 裁剪，超 `MAX_SERIALIZED_MCP_TOOL_BYTES` 时降级为空 object（`C:tools/src/responses_api.rs:121-151`） |
| 动态刷新 | 无（每次重新 `tools/list`） | `notifications/tools/list_changed` → `refresh_after_notifications` 带 revision 检查与 2 次重试（`C:.../client_tool_catalog.rs:171-233`） |
| 注入模型 | **未接线（D-01）** | `append_mcp_tools` → `registry.register_external_with_exposure(handler, ToolExposure::Deferred|Direct|Hidden)`（`C:core/src/mcp_tool_exposure.rs:136-240`，已实读验证） |
| 工具搜索衔接 | `LoadTool` 目录只含内置 Deferred，不含 MCP（`N:.../utils/system_prompt.rs:65-112`） | MCP handler 提供 `ToolSearchInfo`（`C:.../handlers/mcp.rs:191-216`），tool_search 会合并 namespace（`:295-320`） |
| 可见性 | `ensure_server_visible` 按 bundle `enabled_mcp_servers` 过滤三个工具（`N:.../tools/shared/mcp_visibility.rs:9-36`） | `model_can_access_mcp_server` + `EnvironmentMcpPolicy` + orchestrator 开关（`C:.../handlers/mcp_resource.rs:48-63`） |

### 5.5 工具调用链

**Nova**（4 跳，全实读）：
```
mcp_auth {action:"call_tool"} 或 execute_dynamic_with_app("mcp__s__t")
  → [嵌套权限] call_mcp_tool_with_nested_permission   [tools/shared/permission_runtime.rs:71-116]
      → build_mcp_tool_name → enforce_tool_permission → AskUser 时 await_permission_and_recheck
  → mcp::call_mcp_tool                                  [mcp/mod.rs:954-1011]
      → 首次失败 → mark_server_runtime_error → reconnect_server → 再试一次
      → ServerConnection::call("tools/call", {name, arguments})
  → StdioMcpConnection::send_request / StreamableHttp::send_request
  → ToolOutcome::json(Value) 或 ToolFailure::mcp
```

**codex**（6 跳，`C:core/src/tools/handlers/mcp.rs:227-309` → `core/src/mcp_tool_call.rs:157-585` → `C:codex-mcp/src/binding.rs:436-514` → `managed_client.rs:77-115` → `C:rmcp-client/src/rmcp_client.rs:1026-1181`）：
- 调用前用 `prepare_call_for_tool` 在**当前 catalog 快照**里按 `server_name + tool.name + connector_id` 精确定位，保留模型可见名、从当前 catalog 取 schema/annotations/审批元数据（`C:codex-mcp/src/connection_manager/tool_catalog.rs:567-650`）；
- `effective_timeout = min(server_timeout, requested_timeout)`（`binding.rs:445-449`）；
- catalog 快照变更后拒绝调用（"tool call rejected because the catalog changed after..."，`binding.rs:509`）；
- 输出 `CallToolResult{content, structured_content, is_error, meta}`（`binding.rs:517-532`）；
- `sanitize_mcp_tool_result_for_model` 按 `input_modalities` 替换不支持的 image/audio 块（`C:core/src/mcp_tool_call.rs:1075-1112`）；
- `truncate_mcp_tool_result_for_event` 字节级截断（`:1114-1128`）；
- 取消发 `notifications/cancelled`（`C:rmcp-client/src/request_cancellation.rs:38-131`）；
- 审批模板 + 持久化批准（Allow / Allow for this session / Allow and don't ask again / Cancel，`mcp_tool_call.rs:1601-1605`）。

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-5.5 输出截断 | Nova `ToolOutcome::json(v)` 无截断（MCP 专属截断不存在），仅 HTTP 错误体截 240 字符 | 无输出预算概念 | 大结果会整包进模型上下文，直到 90% 才由通用压缩截断；可能一次撑爆上下文 | `N:.../mcp_tools/mod.rs:27-44`、`N:.../tools/mod.rs:97-99`；`C:core/src/mcp_tool_call.rs:1114-1128` |
| D-5.6 模态清洗 | Nova 不做 | 无 `input_modalities` 通道 | MCP 返回 image/audio 块时三种 adapter 各自处理，可能给不支持的模型发图片 | `C:core/src/mcp_tool_call.rs:1075-1112` |
| D-5.7 catalog 一致性 | Nova 调用时按名字现查，无快照校验 | 无 catalog revision | server 中途换工具集时可能调到不存在的工具，错误透传为 String | `N:.../mcp/mod.rs:954-1011`；`C:.../binding.rs:509` |
| D-5.8 审批模板 | Nova 用通用的 `assess_mcp_operation`（按名字启发式） | 未读 MCP annotations | 风险分级不准；无"本次会话允许/永远允许"的 MCP 专用选项 | `N:.../permissions/checks.rs:347-400`；`C:core/src/mcp_tool_approval_templates.rs:53-124`、`C:protocol/src/mcp_approval_meta.rs` |
| D-5.9 取消 | `mcp/` 全目录 grep `cancel|CancellationToken` 零命中；全局回合取消只作用于 LLM HTTP 与流 | MCP 客户端是同步阻塞调用 | 用户点停止后，MCP 子进程/HTTP 请求仍会跑完直到 30s 超时 | `N:.../mcp/mod.rs`、`N:crates/nova-core/src/provider/client.rs:116-133`；`C:rmcp-client/src/request_cancellation.rs:38-131` |

### 5.6 资源、鉴权、elicitation（逐项有/无）

| 能力 | Nova | 依据 | codex |
|---|---|---|---|
| `resources/list`（单 server） | 有 | `N:.../mcp/stdio.rs:132-170`、`mcp/mod.rs:839-884` | 有（`C:.../connection_manager/resources.rs:136-146`） |
| `resources/list`（全 server + 分页） | 无 | 只有单 server 版本 | 有（`resources.rs:45-88`） |
| `resources/templates/list` | **无** | 全库 grep 零命中 | 有（`resources.rs:91-134` + `list_mcp_resource_templates` 工具） |
| `resources/read` | 有 | `mcp/mod.rs:927-942` | 有，含 2026 分支与 elicitation 版（`C:rmcp-client/src/rmcp_client.rs:895-979`） |
| `resources/subscribe` / 通知 | **无** | grep 零命中 | **也无客户端订阅**（仅 `logging_client_handler.rs:74-88` 收通知日志）——这是两边都缺的一项 |
| OAuth 授权码 / PKCE / loopback callback / CIMD·DCR | **无** | 全库 grep `oauth|refresh_token|client_id` 只命中 `docs/agent-architecture-audit.md:495` 的待办 | 有（`C:rmcp-client/src/perform_oauth_login.rs:111-850`、`oauth_client_registration.rs:18-124`） |
| token 存储与刷新 | **无**（仅静态 `headers`，逐条加密落盘） | `N:crates/nova-protocol/src/mcp.rs:37-51`、`mcp/mod.rs:175-221` | keyring / `.credentials.json` + `refresh_if_needed`（`C:rmcp-client/src/oauth.rs:462-510,750-844`） |
| EMA / ID-JAG / 企业 OIDC | **无** | — | 有（`C:rmcp-client/src/ema_auth.rs:35-135`、`enterprise_oauth_login.rs:46-171`） |
| `auth_status` 探测 | 名为 `mcp_auth` 但只是 status/enable/reload/list/call 网关 | `N:.../mcp_auth.rs:31-34` | 有 `McpAuthState{Unsupported,Unknown,LoggedOut,BearerToken,OAuth}`（`C:rmcp-client/src/auth_status.rs:47-120`） |
| elicitation | **无** | 全库 grep 零命中；`send_request` 丢弃非匹配 id | 有 5 类请求分派 + 策略路由（`C:rmcp-client/src/elicitation_client_service.rs:215-310`、`C:codex-mcp/src/elicitation.rs:279-640`） |
| 网络策略 meta 注入 | **无** | 配置结构无字段 | 有，且先剥离调用方伪造再按受信任 capability 注入（`C:.../connection_manager/network_policy.rs:36-85`） |
| 沙箱状态 meta 注入 | **无** | — | 有（`C:codex-mcp/src/sandbox_metadata.rs:14-78`） |
| 连接 lease / 每 chat 隔离 | **无** | 单连接 + 全局 Mutex | 有（`C:codex-mcp/src/connection_pool.rs:258-415`） |

> **`mcp_auth` 命名误导**：它不是鉴权工具，而是 MCP 管理与调用网关（`status/reload_all/enable/disable/list_tools/probe_tool/call_tool`）。真正需要 OAuth 时这个工具名会让维护者误判已有能力。建议后续改名或把鉴权能力补上。

### 5.7 MCP 测试

- Nova：`mcp/` 目录与三个 MCP 工具文件**均无 `#[cfg(test)]`**；MCP 无集成测试、无协议一致性测试、无 stub server。
- codex：`rmcp-client/tests/` 下 37 个相关模块（2026 discovery / stdio / SSE discovery / message limits / MRTR / OAuth discovery / resources / streamable_http_remote / recovery / process_group_cleanup…）、`codex-mcp/src/connection_manager_tests.rs`（267KB）、`core/src/mcp_tool_call_tests.rs`（136KB）、`core/tests/suite/mcp_*` 端到端、`app-server/tests/suite/v2/mcp_*`。
- **影响**：Nova 的 MCP 改动没有任何自动化回归网；D-01 这类"接线漏了"的缺陷正是集成测试最该发现的。

---

## 6. 协议层（逐层比对）

### 6.1 LLM wire API

| 项 | Nova | codex |
|---|---|---|
| 支持形态 | **3 种**：OpenAI Chat Completions (`/v1/chat/completions`)、Anthropic Messages (`/v1/messages`)、OpenAI Responses (`/v1/responses`)，`api_format` 三选一（`N:.../provider/client.rs:49-53,341-361`） | **仅 Responses API**（HTTP `POST /responses` + WebSocket `response.create`），全仓 grep `chat/completions` 零命中（`C:codex-api/src/endpoint/responses.rs:60-69,135-146`、`common.rs:397-403`） |
| 请求体 | 三个 adapter 各自定义 struct（见下） | 单点 `ResponsesApiRequest`（`C:codex-api/src/common.rs:286-314`），字段 `model/stream/service_tier/input/tools/tool_choice/parallel_tool_calls/reasoning/store/stream_options/include/prompt_cache_key/text/client_metadata/access_programs`；**无顶层 `instructions`**，base instructions 作为 `role:"developer"` 的 `ResponseItem` 前插入 `input`（`C:core/src/client.rs:1214-1224`） |
| 默认值 | 各 adapter 不同 | `tool_choice:"auto"`、`store:false`、`stream:true`（`C:core/src/client.rs:1295-1314`） |
| 认证 | openai/responses 仅 `Authorization: Bearer`；anthropic 另加 `x-api-key` + `anthropic-version: 2023-06-01`（`N:.../adapters/*/mod.rs`） | Bearer + `chatgpt-account-id` + `X-OpenAI-Fedramp`；401 后可 `refresh_after_unauthorized`（`C:model-provider/src/bearer_auth_provider.rs:34-49`） |
| beta header | 无 | `OpenAI-Beta: responses_websockets=2026-02-06`（`C:core/src/client.rs:161,176`，常量与测试实读，插入点未实读） |

**三个 Nova 请求体（实读）**：

```rust
// N:.../adapters/openai/types.rs:4-18 + prompt.rs:275-283
OpenAiRequest { model, messages /* 首条 system */, tools: Option<Vec<OpenAiTool>>,
                stream_options: Some({include_usage:true}), stream: true }
// 无 tool_choice、无请求侧 reasoning

// N:.../adapters/anthropic/types.rs:38-52 + prompt.rs:390-403
AnthropicRequest { model, max_tokens, system: Vec<AnthropicSystemBlock{cache_control:ephemeral}>,
                   thinking: Option<budget_tokens>, messages, tools, stop_sequences, stream: true }
// 无 tool_choice

// N:.../adapters/responses/types.rs:4-27 + prompt.rs:277-295
ResponsesRequest { model, input, instructions, tool_choice: Auto, parallel_tool_calls: Some(true),
                   tools, max_output_tokens, truncation: Disabled, reasoning: None,
                   text:{format_type:"text"}, stream: true, prompt_cache_key: "nova:{cid}" }
```

**差异 D-6.1（多协议适配是优势，代价是三份编码）**
- 表现：Nova 覆盖 OpenAI/Anthropic/Responses，codex 绑死 Responses；但 Nova 的 tools 编码三处各写一份：openai 嵌套 `{"type":"function","function":{...}}`（`openai/prompt.rs:260-272`）、responses 顶层扁平 `{"type":"function","name",...}`（`responses/prompt.rs:264-273`）、anthropic 独立 `nova_tools_to_anthropic_tools`。
- 原因：协议差异客观存在，codex 用统一 `ToolSpec` + 单一 wire 规避。
- 影响：① 加字段/改 schema 要改三处；② **Nova 的 Responses adapter 与 codex 的 Responses 请求并不等价**：Nova 发了 `instructions` 顶层字段而 codex 用 developer message 前插；Nova `reasoning: None` 而 codex 带 `Some(reasoning)`；Nova 无 `store`/`include`/`service_tier`/`client_metadata`/`access_programs`。③ Nova 的 `tool_choice`/`parallel_tool_calls` 只在 Responses 下发，另两个协议无此控制。
- 证据：`N:.../provider/client.rs:49-53,341-361`、`N:.../adapters/{openai,anthropic,responses}/{types.rs,prompt.rs}`；`C:codex-api/src/common.rs:286-314`、`C:core/src/client.rs:1214-1224,1295-1314`。

### 6.2 tool-calling 编码与 id 关联

| | Nova（内部统一表示 `N:crates/nova-protocol/src/llm.rs:71-89`） | codex |
|---|---|---|
| 内部 | `ContentBlock::ToolUse{id,name,input}` / `ToolResult{tool_use_id,is_error,content}` | `ResponseItem::FunctionCall{call_id, arguments:String, id:Option<ResponseItemId>, namespace}` / `FunctionCallOutput{call_id:Option<String>, output:FunctionCallOutputPayload}`（`C:protocol/src/models.rs:1141-1208`） |
| openai | `OpenAiReqToolCall{id,type:"function",function{name,arguments:String}}`；结果 `role:"tool", tool_call_id`（`openai/types.rs:20-42`、`prompt.rs:145-196`） | 不支持 |
| anthropic | `ContentBlock::ToolUse{id,...}`；结果 `tool_use_id` + `is_error`（`anthropic/types.rs:101-110`、`prompt.rs:63-100`） | 不支持 |
| responses | `function_call{call_id,name,arguments:String}`；结果 `function_call_output{call_id,output:String}`（`responses/types.rs:38-46`、`prompt.rs:82-168`） | `ToolSpec` 5 形态；`create_tools_json_for_responses_api`（`C:tools/src/tool_spec.rs:82-93`）；`ResponsesApiTool.output_schema` 带 `#[serde(skip)]` 不上 wire |
| 结果内容 | `responses` 的 `output` 是 **String**，含图片/错误时序列化成 `{"is_error":...,"content":[...]}` JSON 字符串（`responses/prompt.rs:115-128`） | `FunctionCallOutputPayload` 支持 `content` 字符串或 `content_items` 结构化数组（`C:protocol/src/models.rs:1185-1208`） |

**差异 D-6.2（结构化内容降级为字符串）**
- 表现：Nova 的 `Read` 图片通过 `additional_messages` 侧信道回灌，但 Responses adapter 的 tool output 是字符串；codex 用 `FunctionCallOutputContentItem::InputImage` 等结构化项。
- 原因：`FunctionCallOutputPayload` 未在 Nova 侧建模。
- 影响：多模态工具结果在 Responses 协议下丢失类型信息，模型只能看到一段 JSON 文本。
- 证据：`N:.../adapters/responses/prompt.rs:82-129`；`C:core/src/tools/handlers/view_image.rs:238-245`、`C:protocol/src/models.rs:1185-1208`。

### 6.3 SSE 与增量聚合

| 项 | Nova | codex |
|---|---|---|
| 帧 | `sse_utils.rs:22-46` 按 `\n\n`/`\r\n\r\n` 分帧、`data:` 多行拼接；`[DONE]` 跳过 | `codex-client/src/sse.rs:9-48` 用 `eventsource_stream::Eventsource`，idle timeout 触发 `StreamError::Timeout` |
| 事件类型 | openai 无类型枚举（直接解 `OpenAiStreamChunk{choices,usage,error}`）；anthropic `StreamEvent` 8 变体 + `Other`（`anthropic/types.rs:148-193`）；responses 22 变体 + `Other`（`responses/types.rs:113-273`，含"非标准扩展 OpenRouter"注释） | `ResponsesStreamEvent`（`C:codex-api/src/sse/responses.rs:161-179`）+ `process_responses_event:375-559` 分派，`ResponseEvent` 18 变体（`common.rs:81-135`）；**显式忽略** `response.function_call_arguments.delta/.done`（`:538-550`） |
| 增量聚合 | adapter 侧 `BTreeMap<usize,PendingToolCall>`（openai/mod.rs:16-24,195-309）、`pending_fn_calls`（responses/mod.rs:18-35）、标量状态（anthropic/mod.rs:18-29）→ 统一 `Delta` 10 种（`stream_runner.rs:64-92`）→ `AssistantOutputBuilder`（`:124-200`）→ `process_delta`（`:613-829`） | **不在 SSE 层聚合**，下游 `session/turn.rs:3608-3656` 用 `active_item` + `active_tool_argument_diff_consumer` 把 delta 转成 `EventMsg`；patch 参数有 500ms 缓冲的 `ApplyPatchArgumentDiffConsumer` |
| wire 轨迹 | `SessionEvent::WireRequest/WireResponse` 落库（`client.rs:95-114`、`stream_runner.rs:512-530`），供 `render_turn_traces` 展示（**面试可讲的亮点**） | rollout + otel `log_sse_deltas` 开关 |

### 6.4 错误分类与重试（最影响可靠性的一组）

**Nova 的错误判定分散在三处，全是字符串匹配**：
```rust
// N:.../provider/stream_runner.rs:290-332  流 chunk 分类
err_str.contains("error decoding response body") || ... || "stream reading error" || "wsarecv" ...
// N:.../provider/adapters/openai/mod.rs:91-103  socket cut 类 → Delta::StreamWarning（容忍）
// N:.../capabilities/compact/summary.rs:127-141  is_prompt_too_long_error
"prompt_too_long" | "prompt too long" | "context length" | "context too long"
  | "maximum context length" | "context window" | "too many tokens" | "token limit exceeded"
```
HTTP 非 2xx：`client.rs:137-148` → `ProviderTurnError::new`，**无重试**。空闲看门狗 `STREAM_IDLE_TIMEOUT=180s`（`stream_runner.rs:216,247-256`）、`read_timeout=120s`、`connect_timeout=10s`（`client.rs:55-66`）。provider 目录 grep `RetryState|retry_state|MAX_RETRIES|attempt` **零命中**。

**codex**：`codex-api/src/sse/responses_error.rs:26-105` 按 `error.code` 映射到 `ApiError`（`ContextWindowExceeded`/`QuotaExceeded`/`CyberPolicy`/`BioPolicy`/`MisalignmentPolicyViolation`/`RateLimitExceeded`…），`RateLimitExceeded` 还能从 message 正则提取 retry-after（`:108-146`）；`core/src/responses_retry.rs:32-46` `ResponsesStreamRetryState{retries,connection_retries,connection_retry_delay}`，`handle_response_stream_error:58-177` 依次做：content-filter 引导 → 终态判定 → 无界连接重连 → **WebSocket 回退 HTTPS 并重置 retries** → 常规退避 → 耗尽写 `ExhaustedResponseRetry`；`stream_max_retries` 默认 5 / 上限 100（`C:model-provider-info/src/lib.rs:73-82,550-555`）；传输层另有 `RetryPolicy`（429/5xx/transport + jitter，`C:codex-client/src/retry.rs:8-52`）。

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-03 错误分类 | Nova 字符串 `contains()`；codex 类型化枚举（内部 `CodexErrorDetails` → 对外 `CodexErrorInfo` 22 变体，已实读验证 `C:app-server-protocol/src/protocol/v2/shared.rs:73-131`） | 全链路 `Result<T,String>`（Nova 304 处） | Nova 无法按错误类别决策：限流不能退避、超限只能靠字符串猜、鉴权失败与网络断流都汇成 `TurnError{error:String}`（唯一出口 `handle_turn_error`，`engine.rs:742-769` 16 行） | `N:.../engine.rs:742-769`、`N:.../provider/stream_runner.rs:290-332`；`C:protocol/src/error.rs:104-220`、`C:.../v2/shared.rs:73-197` |
| D-04 重试状态机 | Nova provider 层零重试；只有 RPC 层 `-32001` 指数退避（最多 5 次，`nova-client/src/lib.rs:188-207`）、压缩摘要重试 2 次、MCP 重连一次 | 没有"可恢复 vs 终态"的分类 | 一次 SSE 中途断流 = 整轮失败；codex 会按 `will_retry` 通知客户端（`C:.../v2/notification.rs:59-69` 的 `ErrorNotification.will_retry`）并自动重连/换传输 | `N:.../stream_runner.rs:290-332`、`N:.../nova-client/src/lib.rs:188-207`；`C:core/src/responses_retry.rs:58-177` |
| D-6.3 超限处理 | Nova 反应式：`is_prompt_too_long_error` → `reactive_compact_messages_for_retry` → 重发一次 | 服务端错误码未结构化 | 若供应商错误文案不在 8 个关键词里（例如中文报错、HTTP 400 带 JSON code），直接整轮失败 | `N:.../engine.rs:334-371`、`N:.../compact/summary.rs:127-141`；`C:codex-api/src/sse/responses_error.rs:26-105` |
| D-6.4 token 估算 | Nova 全项目统一 o200k_base BPE，**不按模型/协议分支**（`token_counter.rs:1-3`），真实 usage 覆盖估算；`pricing.rs:19-53` 算成本 | 简化 | Anthropic/DeepSeek 的 tokenizer 与 o200k 不同，压缩阈值（80%/90%）会有偏差；codex 用服务端 usage + `ContextWindowTokenStatus` | `N:.../provider/token_counter.rs:1-53`、`pricing.rs:19-53`；`C:core/src/session/context_window.rs:7-118` |

### 6.5 客户端 RPC / IPC

| 项 | Nova | codex |
|---|---|---|
| JSON-RPC | **严格 2.0**：`JsonRpcRequest{jsonrpc,id,method,params}` / `Response` / `Error` / `Notification`（`N:crates/nova-protocol/src/rpc.rs:4-50`），`PROTOCOL_VERSION="1"`（`:54`） | **非严格 2.0**：`app-server-protocol/src/rpc.rs:1-2` 明确 "We do not do true JSON-RPC 2.0, as we neither send nor expect the `jsonrpc: 2.0` field"；`JSONRPCMessage` untagged 4 变体、`RequestId` = string\|int |
| 握手 | `initialize` 校验 `protocolVersion`，不匹配回 `-32600`（`serve/mod.rs:315-336`）；`ClientCapabilities{browser,hooks}` / `ServerCapabilities{browser,hooks,escalation}` 结构化（`rpc.rs:60-112`） | v1 `InitializeParams{client_info,capabilities}` → `InitializeResponse{user_agent,codex_home,platform_family,platform_os}`（`v1.rs:27-88`）；v2 转 thread/turn/item |
| 方法数 | dispatch 89 个方法分支（实测）、Tauri command 100 个（实测）、`src/lib.rs:34-135` 注册约 100 个 | `app-server-protocol/src/protocol/common.rs` 宏生成 `ClientRequest`（实测约 282 个 `=> "..."` 行）+ `server_notification_definitions!`（`:2043-2145+`） |
| 生命周期 | 无 thread/item；`SessionEvent::TurnStart/TurnEnd` + `AgentDomainEvent{TurnStarted,TurnFinished,TurnError}`（`N:crates/nova-protocol/src/agent.rs:7-74`） | `thread/started` → `turn/started` → `item/started` → 各类 `*/delta` → `item/completed` → `turn/completed`（`C:.../common.rs:2043-2087`） |
| Item 模型 | 无。`HistoryToolExecution{id,status running/completed/error/cancelled}`（`nova-protocol/src/session.rs:66-86`），upsert 在 `projection.rs:388-446` 按 `call_id` 配对 | `ThreadItem{payload(flatten), started_at_ms, completed_at_ms}`，`id`/`status` 在各 `ThreadItemPayload` 变体内（`C:.../v2/item.rs:276-341`）；同 id 的 started+delta+completed 构成覆盖 |
| history | append-only 事件日志 → `render_ui_history`/`render_tool_logs`/`reconstruct_model_context`/`render_turn_traces` 四投影 | rollout 逐行投影 `ThreadHistoryChangeSet{changed_turns,changed_items}`（`C:.../thread_history_projection.rs:17-99`）+ `TurnItemsView{NotLoaded,Full,Summary}` |
| 限流 | `Semaphore(32)`，超限回 `-32001`（`serve/mod.rs:35-36,394-404`）；客户端对 `-32001` 退避重试 | `-32001 OVERLOADED` 定义在 `C:app-server/src/error_code.rs:3-8` |
| 错误码 | `-32600` 版本 / `-32601` 方法 / `-32602` 参数 / `-32603` 内部 / `-32001` 过载 / `-32000` 客户端通道 | `-32600/-32601/-32602/-32603/-32001` + `input_too_large` |

**差异 D-07（事件流 vs Item 集合）**
- 表现：Nova 前端是"顺序消费事件的状态机"（`src/features/agent/agent-listener.ts:24-89` 按 10 类 `kind` 分发），codex 前端是"Item 集合渲染"。
- 原因：Nova 事件对象没有可 upsert 的稳定 id 与状态生命周期。
- 影响：丢一条 `ToolCallCompleted` → UI 永久停在 running，只能全量重拉 `load_conversation_tool_logs`；codex 重放一条 `item/completed` 即自愈。**注意**：Nova 的投影层已经实现了按 `call_id` 的终态配对与去重（`projection.rs:388-446`），缺的不是投影能力，而是把"增量重放/收敛"暴露成客户端可调用的协议。

### 6.6 TS 绑定与契约校验

| 项 | Nova | codex |
|---|---|---|
| 生成 | `scripts/gen-bindings.js:14-28`：`cargo test -p nova-protocol` + `TS_RS_EXPORT_DIR=src/lib` 触发 ts-rs 导出，产出 `src/lib/protocol.ts`；只校验文件存在 | `app-server-protocol/src/export.rs:45-81` + `precomputed_exports.rs:14-90`：ts-rs `#[ts(export_to="v2/")]` + schemars → `generate_ts` / `generate_json` → prettier → 落盘 `schema/typescript/`、`schema/json/`、`*.json.zst` |
| 校验 | `scripts/check-contract.js:57-103`：正则扫前端 `invoke('...')` ↔ `dispatch.rs` 的 `"xxx" =>` ↔ `api/*.rs` 的 `pub fn`，**只比方法名集合**；有 17 个 `DESKTOP_LOCAL` 与 4 个 `INTERNAL_API` 白名单 | 三重建：`precomputed_exports_tests.rs:13-42`（预计算导出 == 实时生成）、`schema_fixtures_tests.rs:18-55`（fixture 树 == 生成树）、`sdk/python/tests/test_contract_generation.py`（生成文件字节快照） |
| JSON schema | **无**（`nova-protocol` 未生成 JSON schema） | 有（`schema/json/*.json`，含 `codex_app_server_protocol.v2.schemas.json`） |
| 历史残留 | `crates/nova-protocol/bindings/protocol.ts` 未被 git 跟踪，21 行、写着"与 Codex App Server 的握手同形"，`capabilities: Array<string>`，与当前 `rpc.rs:60-81` 的结构化 capabilities **冲突** | 无 |
| MCP 类型 | `nova-protocol/src/mcp.rs` 只有 `#[derive(TS)]`，**无 `#[ts(export, export_to="protocol.ts")]`**，所以 `src/lib/protocol.ts` 里没有 MCP 类型；前端 `McpServerStatus` 是手抄的（`src/components/agent/AgentConfigScreen.vue:38`） | v2/mcp.rs 完整导出 |

**差异 D-08（契约校验不覆盖类型）**
- 表现：改 Rust 字段名/类型、或 ts-rs 未重新生成，只要方法名不变，`check:contract` 依然通过。
- 原因：脚本用正则提取标识符，不做 schema 比对。
- 影响：`src/lib/protocol.ts` 与 Rust 可能静默漂移；MCP 类型完全没有生成物、靠前端手抄；`bindings/protocol.ts` 是过期残留，容易误导。
- 证据：`N:scripts/check-contract.js:57-103`、`N:scripts/gen-bindings.js:14-28`、`N:crates/nova-protocol/src/mcp.rs`（grep `ts(export` 零命中）、`N:crates/nova-protocol/bindings/protocol.ts:9-21`、`N:src/components/agent/AgentConfigScreen.vue:38`；`C:app-server-protocol/src/{precomputed_exports_tests.rs:13-42,schema_fixtures_tests.rs:18-55}`。

### 6.7 传输帧

| 通道 | Nova | codex |
|---|---|---|
| stdio | NDJSON 一行一 JSON，`BufReader.lines()` + writer 协程 `to_vec + \n + flush`（`serve/mod.rs:434-494`） | NDJSON（无 Content-Length），`transport/stdio.rs:73,138-149` |
| WebSocket | `/ws` 只处理 `Message::Text`，出站 `Message::Text`（`serve/mod.rs:530-565`） | Text 帧承载 JSON，或 Zstd 压缩的 Binary（`transport/websocket.rs:283-390`） |
| UDS | **无** | UDS 上跑 WebSocket 升级 + Zstd 协商（`transport/unix_socket.rs:193-245`） |
| 反向能力调用 | `/capabilities/browser` 请求-响应（120s 超时）、`/capabilities/hooks` 通知（`serve/mod.rs:170-241`） | 扩展工具 / `client_tool_catalog` / dynamic tools |
| 事件出站 | `event_pump` → `{"jsonrpc":"2.0","method":"event","params":{name,payload}}`；队列满**丢一条并告警**（`serve/mod.rs:248-272`） | Item/delta 通知，可重放 |
| 桥接 | 无 | `stdio-to-uds/src/lib.rs:11-40` 裸字节中继；`tcp-tunnel/src/control.rs:1-28` 单行 NDJSON 元数据 |

---

## 7. 接口定义

### 7.1 三套接口的实际形态

| 接口面 | Nova | codex |
|---|---|---|
| 内核 RPC | `nova-cli/src/serve/dispatch.rs:22-157` 一个大 `match`，**实测 89 个**方法分支；`arg<T>(&params,"camelCaseKey")` 取参并反序列化；错误码 `-32601/-32602/-32603`（`:11-20,160-175`） | `app-server-protocol/src/protocol/common.rs` 的 `client_request_definitions!` / `server_notification_definitions!` 宏生成，实测约 282 个 `=> "..."` 行；v1 只保留 3 个遗留方法（`export.rs:77-78`） |
| 桌面 command | `src-tauri/src/lib.rs:34-135` 注册约 100 个，`src-tauri/src/command/*.rs` 全部是薄壳 `daemon::invoke("方法名", json!({"camelCase":...}))`（例 `command/mcp.rs:8-72`、`command/agent.rs:3-19`），**实测 100 个 `#[tauri::command]`** | 无 Tauri 层；app-server 统一对外 |
| 事件/通知 | `NovaEvent` 13 变体 → `to_frontend()` 11 个事件名（`N:crates/nova-core/src/events.rs:16-77,82-194`）；`AgentDomainEvent` 10 变体（`nova-protocol/src/agent.rs:7-74`） | `ServerNotification` 宏生成（`thread/started`/`turn/*`/`item/*`/`item/agentMessage/delta`/`item/commandExecution/outputDelta`…） |
| 类型契约 | `src/lib/protocol.ts`（ts-rs 生成，头部 `Do not edit this file manually`）；已实读比对：`AgentDomainEvent` 10 变体、`CognitiveState` 9 态、`ApprovalPolicy` 3 值、`ContentBlock` 5 变体、`AppSettings` 14 字段均一致 | `schema/typescript/index.ts` + `v2/` + `schema/json/*.json` + `*.json.zst` 预计算导出 |

### 7.2 接口层差异

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-7.1 方法名字符串分发 | Nova 89 个方法靠字符串 `match` + 手写 `arg()` 取参；codex 宏生成带类型化的 `ClientRequest` 枚举 | 轻量实现 | 加接口要同时改 4 处（api / dispatch / tauri command / 前端 invoke），靠 `check-contract.js` 正则兜一致性；参数类型不匹配要到运行时 `-32602` 才发现 | `N:.../serve/dispatch.rs:22-175`、`N:scripts/check-contract.js:57-103` |
| D-7.2 参数命名 camelCase 约定 | Rust 侧 `arg(&params,"conversationId")`，Tauri 侧 `json!({"conversationId":...})`，靠约定对齐而非共享类型 | 跨进程 JSON 边界 | `check-contract.js` 只比方法名，键名写错无静态检查 | `N:src-tauri/src/command/agent.rs:8-12`、`N:.../serve/dispatch.rs:26-33` |
| D-7.3 无版本化协议 | Nova `PROTOCOL_VERSION="1"` 只做相等校验（不符回 `-32600`），无 v1/v2 兼容层、无实验方法标记 | 单客户端同时发布 | 桌面壳与内核版本错配时只能整体拒绝；codex 有 v1 白名单 + `#[experimental]` 标记 + v1→v2 mappers | `N:.../serve/mod.rs:315-336`；`C:app-server-protocol/src/protocol/{v1.rs:27-130,mappers.rs:3-24}`、`export.rs:77-78` |
| D-7.4 能力协商 | Nova `ClientCapabilities{browser,hooks}` / `ServerCapabilities{browser,hooks,escalation}` 结构化，`initialized` 是火忘式不等响应（`nova-client/src/lib.rs:154-181`） | 桌面反向能力需要注册 | `initialized` 不等响应意味着内核侧能力注册完成的时机对客户端不可见 | `N:crates/nova-protocol/src/rpc.rs:60-112`、`N:crates/nova-client/src/lib.rs:154-181` |

---

## 8. 依赖配置

### 8.1 workspace 与依赖实测

| crate | 主要依赖（实读各 `Cargo.toml`） |
|---|---|
| `nova-protocol` | serde / serde_json / **ts-rs 12**（`serde-json-impl`） |
| `nova-core` | nova-protocol；**jsonschema 0.46.5**、ignore/glob/globset、grep-regex/grep-searcher、similar 2.7、**reqwest 0.13.2(json,stream)**、**sqlx 0.8.6(sqlite,runtime-tokio,macros)**、image 0.24、lopdf、aes-gcm、rand、tracing 0.1.44、url/urlencoding、croner 3、**tiktoken-rs 0.12**、**tree-sitter 0.25 / tree-sitter-bash 0.23**、libc、dirs；Windows: winreg 0.10 / **windows-sys 0.61** |
| `nova-client` | nova-protocol、serde_json、tokio（stdio JSONL） |
| `nova-cli` | clap 4.5、colored、inquire 0.7、**ratatui 0.29**、crossterm 0.28、arboard、**axum 0.8(ws)**、fs2 0.4、tracing + subscriber + appender |
| `src-tauri` | **tauri 2(unstable)**、tauri-plugin-dialog/opener、webview2-com 0.39、nova-protocol + nova-client（**不依赖 nova-core**）、portable-pty 0.9 |
| 前端 | Vue 3.5、pinia 3、vue-router 4、@tauri-apps/api 2.12、echarts、markdown-it + katex、xterm、tailwind 4、vite 6、vue-tsc 2.1；**devDependencies 无任何测试框架** |

`N:Cargo.toml:1-8` 5 members、resolver 2、edition 2021；`[workspace.dependencies]` 只统一了 7 个基础 crate（serde/serde_json/tokio 1.50/uuid 1.23/chrono 0.4.44/thiserror 2/anyhow 1.0.102）。

### 8.2 codex 侧

124 个 crate，功能面拆得极细：`codex-mcp` / `rmcp-client` / `sandboxing` / `execpolicy` / `hooks` / `tools` / `rollout` / `state` / `thread-store` / `app-server-protocol` / `codex-api` / `model-provider` / `otel` / `windows-sandbox-rs` / `message-board-mcp` / `code-mode-*` / `plugin-*` 等；另有 Bazel 构建（`BUILD.bazel`、`bazel/`）、`deny.toml`、`rust-toolchain.toml`、`config-schema`、`patches/`、`third_party/`。

### 8.3 差异

| 差异点 | 表现 | 原因 | 影响 | 证据 |
|---|---|---|---|---|
| D-8.1 关键依赖缺失 | Nova **无 `rmcp` SDK**（手写 JSON-RPC）、**无 otel**、**无 insta/快照测试**、**无前端测试框架**；codex 用 `rmcp-client` 承接全部 MCP 协议演进 | 最小依赖策略 | 直接导致 §5 的协议代差与 §10 的契约测试缺口 | `N:crates/nova-core/Cargo.toml`、`N:package.json`；`C:rmcp-client/Cargo.toml` |
| D-8.2 三协议适配的代价 | Nova 要同时维护 openai/anthropic/responses 三套 `types.rs` + `prompt.rs` + `mod.rs`（各约 300–470 行） | 支持 DeepSeek(OpenAI 兼容) + Anthropic + Responses | 项目记忆中"已排除 Anthropic"是产品决策，但代码里仍保留完整 anthropic adapter（`provider/adapters/anthropic/`），属**未清理的并行实现**，增加维护面 | `N:crates/nova-core/src/provider/adapters/` |
| D-8.3 构建与发布 | Nova `package.json` 有 `sync:cli`/`gen:bindings`/`check:contract` 三脚本 + 多平台 tauri build；已有 `release-artifacts/`（3 exe + 1 msi） | 桌面分发 | codex 有 Bazel + 多平台沙箱二进制，Nova 无 CI 配置（本次未见 `.github/workflows`） | `N:package.json:5-18`、`N:release-artifacts/` |
| D-8.4 单 crate 内核 | 14107 行 `nova-core` 一个 crate | 单人项目追求编译速度与改动半径 | 优点：改一处即可；缺点：MCP/沙箱/协议无法独立版本化与测试 | `N:crates/nova-core/src/` |

---

## 9. 错误处理

### 9.1 两套体系的实证对照

| 层 | Nova | codex |
|---|---|---|
| 通用返回 | `Result<T, String>` **304 处**（实测 `grep`）；`execute_turn`/`assemble`/`load_system_prompt`/所有 `api/*`/session 层全用它 | `CodexErr{ details: CodexErrorDetails, retry_after, agent_context }`（`C:protocol/src/error.rs:78-170`），`CodexErrorDetails` 有 30+ 语义变体，含 `TurnAborted/SessionBudgetExceeded/RateLimitExceeded/ContextWindowExceeded/ModelNotFound{status}/Sandbox/…` |
| 自定义错误枚举 | **仅 2 个 thiserror 派生枚举**，都在 `apply_patch`：`file_update.rs:11 ApplyPatchError`、`parser.rs:17 ParseError` | 多层枚举：`ApiError`（`codex-api/src/error.rs:10-73`）、`FunctionCallError`（`tools/src/function_call_error.rs:4-10`）、`ToolError`（`core/src/tools/sandboxing.rs:376-380`）、`CodexErrorInfo`（v2，22 变体） |
| 工具错误 | `ToolFailureKind` 7 种（`tools/mod.rs:107-126`）但只有 `message:String` 载荷；`kind` 主要决定是否发 `emit_backend_error`（`:348-359`） | `RespondToModel(String)` vs `Fatal(String)` 直接决定"回给模型重试"还是"终止"；`ToolError::Rejected` vs `Codex(CodexErr)` 决定编排器是否升级重试 |
| 客户端错误 | JSON-RPC code `-32600/-32601/-32602/-32603/-32001/-32000`；`ErrorNotification` 不存在，只有 `AgentDomainEvent::TurnError{error:String}` | `ErrorNotification{error:TurnError, will_retry:bool, thread_id, turn_id}`（`C:.../v2/notification.rs:59-69`）+ `CodexErrorInfo` + `ModelProviderError`（v2/model_provider_error.rs:8-38） |
| 可重试判定 | 无 | `retry_delay(retry_count)` 对大批变体返回 `None`（终态），其余带 jitter 退避（`C:protocol/src/error.rs:426-470`）；`retry_after()` 从 message 提取 |
| 错误广播 | `emit_backend_error`（`utils/error_event.rs:17-48`）：tracing::error + `NovaEvent::BackendError` + eprintln；`report_backend_result`（`:50-60`）包装 api 层 | 结构化事件 + otel + `ErrorEvent{codex_error_info, rejected_audio_file_ids, misalignment}` |

### 9.2 差异的四要素

**D-03 错误类型化**
- **表现**：Nova 从工具失败、HTTP 非 2xx、流中断、上下文超限、审批拒绝到 MCP 错误，最终都变成 `String`；唯一下游分支是 `is_prompt_too_long_error(&err_msg)` 的字符串匹配与 `ToolFailureKind` 的是否发错误事件。
- **原因**：全链路用 `String` 作为错误载体，没有引入领域错误枚举；三协议适配进一步放大了"错误文案来自不同供应商"的多样性。
- **影响**：① 无法区分限流/鉴权/超限/网络断流，因此无法实现分类退避或 `will_retry`；② 供应商改一句错误文案就使压缩自愈失效；③ 前端只能展示一个字符串，无法给出"重试 / 换模型 / 检查 API Key"这类可操作提示；④ `write_plan` 甚至把错误包成成功（§4.2.4 ⑮），错误信号在工具层就丢失了。
- **证据**：`N:.../engine.rs:334-371,742-769`、`N:.../provider/stream_runner.rs:290-332`、`N:.../capabilities/compact/summary.rs:127-141`、`N:.../tools/mod.rs:107-175,348-359`、`N:.../tools/write_plan_tool/write_plan.rs:86-97`；`C:protocol/src/error.rs:78-170,426-470,530-595`、`C:app-server-protocol/src/protocol/v2/shared.rs:73-197`。

---

## 10. 测试覆盖

### 10.1 规模实测

| 指标 | Nova | codex |
|---|---|---|
| `#[cfg(test)]` 模块 | **33** | 1675 个文件含 `#[cfg(test)]` |
| `#[test]` / `#[tokio::test]` | **106** | **20129** |
| `tests/` 集成测试目录 | **0** | **62** |
| 前端测试 | **0**（无 vitest/jest，`package.json` devDependencies 无测试框架） | TypeScript SDK `sdk/typescript/tests/`、Python SDK 契约测试 |
| 契约/快照测试 | **0**（`check-contract.js` 是正则方法名比对，非快照） | `schema_fixtures_tests.rs`、`precomputed_exports_tests.rs`、`test_contract_generation.py` |
| MCP 专属测试 | **0** | `rmcp-client/tests/` 37 模块 + `connection_manager_tests.rs`(267KB) + `mcp_tool_call_tests.rs`(136KB) + `core/tests/suite/mcp_*` + `app-server/tests/suite/v2/mcp_*` |

### 10.2 Nova 已覆盖与盲区

**已覆盖**（全为内联单测、纯函数）：`ToolLoopBreaker`（3 警告/5 熔断/重置）、`ContextAssembler`（旧快照剥离/角色合并）、compact 截断与合法 JSON、`permissions` 规则精确/前缀/deny 优先、bash_ast 路径与 PowerShell 判定、sandbox 策略与 bubblewrap 参数、hooks TOML/matcher/阻断、board/graph BFS、projection 块时序、`apply_patch` 解析/模糊匹配/CRLF、Read 图片真解码、WebFetch HTML→MD、WebSearch 各引擎解析与域名过滤、`nova-client` 2 万条事件零丢失、三个 adapter 的 SSE 解析、`token_counter` CJK/图片/预算。

**盲区**（无自动化测试，按风险排序）：
1. **MCP 全链路**（正是 D-01 藏身之处）：`collect_mcp_tools` 是否被调用、握手、工具注入、调用。
2. **engine 端到端 turn**：`execute_turn` 614 行无任何集成测试；工具回灌顺序、取消路径、重放收敛都靠人工。
3. **权限审批链路**：`AskUser → await_permission_decision → apply_decision → AllowAlways 持久化` 无测试。
4. **沙箱实际执行**：只测了参数拼装，`run_once_unsandboxed` 与 Windows 驱动未测。
5. **hooks 与外部进程集成**：只测配置与 dispatch 纯逻辑，`command.rs` 的真实子进程退出码语义未测。
6. **Tauri ↔ daemon ↔ 前端契约**：方法名靠正则脚本，字段类型零校验。

### 10.3 差异四要素（D-10）

- **表现**：Nova 106 个测试全部是内联单测且集中在纯函数；codex 有 62 个 `tests/` 目录、端到端 suite、协议一致性测试与生成物快照。
- **原因**：单人项目优先保证"算法级正确"，没有为跨进程/跨协议链路搭测试夹具（无 MCP stub server、无 SSE fixture、无 headless 端到端 harness）。
- **影响**：**结构接线类缺陷无法被自动发现**——D-01 就是典型：`collect_mcp_tools` 本身若有单测也会通过，因为它没有调用点。同理 `compact_messages_for_turn`（休眠代码）、`write_plan` 把错误当成功、`ALWAYS_ON_TOOLS` 大小写不匹配、`spawn_agent` 丢弃 role，全部是"函数写对了但集成没对"的问题。
- **证据**：`N:crates/nova-core/src/agent/engine.rs:772-868`（唯一 engine 测试只测 `ToolLoopBreaker`）、`N:package.json`（无测试框架）、`N:scripts/check-contract.js`；`C:codex-api/src/sse/responses.rs:789-998`（SSE 快照断言）、`C:app-server-protocol/src/schema_fixtures_tests.rs:18-55`。

---

## 11. 主要差异结论矩阵

按"对用户可观察行为的影响"排序。`Nova 独有`表示 codex 无对等物，不是差距。

| ID | 域 | 差异结论 | 性质 | 严重度 | 主要证据 |
|---|---|---|---|---|---|
| D-01 | MCP | MCP 工具定义未接入模型清单，模型只能经 `mcp_auth` 网关间接调用 | 接线缺失 | **P0** | `N:.../mcp_tools/mod.rs:78`、`N:.../tools/mod.rs:708-790` |
| D-02 | MCP | 协议固定 2024-11-05、SSE stub、无 OAuth/elicitation/池化/取消/模板/订阅 | 协议代差 | **P0** | `N:.../mcp/stdio.rs:263-290`、`sse.rs:1-5` |
| D-03 | 错误 | `Result<T,String>` 304 处 + 字符串判错 vs 类型化错误体系 | 控制流基础 | **P0** | `N:.../engine.rs:742-769`；`C:protocol/src/error.rs` |
| D-04 | 协议 | 无流式重试状态机，无 `will_retry` | 可靠性 | **P0** | `N:.../stream_runner.rs:290-332`；`C:core/src/responses_retry.rs` |
| D-05 | 工具 | 28 个静态 `ToolRegistration` vs 5 种 `ToolSpec` + 动态注册 | 架构 | P1 | `N:.../tools/mod.rs:35-64`；`C:tools/src/tool_spec.rs:20-67` |
| D-06 | 内核 | 审批/沙箱/重试分散三层 vs `ToolOrchestrator` 泛型编排 | 架构 | P1 | `N:.../tools/mod.rs:822-881`；`C:.../orchestrator.rs:125-646` |
| D-07 | 协议 | 事件流状态机 vs Item id/status upsert；无增量重放 RPC | 客户端健壮性 | P1 | `N:.../session/projection.rs:388-446`；`C:.../v2/item.rs:276-341` |
| D-08 | 接口 | 契约只校验方法名集合，无 JSON schema / 快照；MCP 类型未导出 | 契约漂移 | P1 | `N:scripts/check-contract.js:57-103` |
| D-09 | 内核 | 单 614 行 `execute_turn`，无三层上下文对象、无 steer/排队/步级 timing | 可扩展性 | P1 | `N:.../engine.rs:128-740` |
| D-10 | 测试 | 106 内联单测 / 0 集成测试 / 0 前端测试 | 质量保障 | P1 | 实测统计 |
| D-11 | 工具 | 无 PTY 会话（`Bash`/`BashOutput` 轮询日志 vs `exec_command`/`write_stdin`） | 功能深度 | P1 | `N:.../bash_tool/bash.rs`；`C:.../unified_exec/` |
| D-12 | 沙箱 | 默认 `DangerFullAccess`，Windows 驱动实现未被 `active_driver()` 选用 | 安全 | P1 | `N:.../sandbox/mod.rs:24-37,277-280` |
| D-13 | 协议 | Nova 支持 3 种 LLM 协议（**Nova 独有优势**），但三份 tools 编码、Responses 请求与 codex 不等价 | 取舍 | P2 | `N:.../provider/client.rs:49-53` |
| D-14 | 工具 | `ToolLoopBreaker` 熔断（**Nova 独有优势**） | 取舍 | — | `N:.../engine.rs:21-85` |
| D-15 | 权限 | tree-sitter-bash AST 静态分析深度（**Nova 局部优于** codex 规则匹配） | 取舍 | — | `N:.../permissions/bash_ast/` |
| D-16 | 协议 | `WireRequest/WireResponse` 轨迹 + 4 个投影（**Nova 独有优势**） | 取舍 | — | `N:.../session/projection.rs:488-528` |
| D-17 | MCP | `mcp_auth` 命名与能力不符；名为 auth 实为管理网关 | 可维护性 | P2 | `N:.../mcp_auth_tool/mcp_auth.rs:31-34` |
| D-18 | 工具 | 4 处"写对了但没接对/没用对"：`WritePlan` 大小写、`write_plan` 错误当成功、`spawn_agent` 丢弃 role、`compact_messages_for_turn` 休眠 | 缺陷 | P1 | `N:crates/nova-protocol/src/misc.rs:80,92-95`、`write_plan.rs:86-97`、`spawn_agent.rs:57-65`、`compact/mod.rs:814,865` |
| D-19 | 内核 | 多智能体单版本 + 进程内 board/graph，无持久化/异步协作/深度计数 | 功能深度 | P2 | `N:.../orchestration/` |
| D-20 | 依赖 | 无 `rmcp` SDK / otel / 快照测试 / 前端测试框架 | 工程 | P2 | `N:crates/nova-core/Cargo.toml`、`package.json` |

### 11.1 三条归纳

1. **四个 P0 是同一条根因链的四个投影**：错误没有类型 → 无法判定"可恢复/终态" → 没有重试状态机 → 断流即整轮失败；而 MCP 侧没有 SDK → 协议版本固定 → capabilities 不读 → 工具注入/鉴权/取消都无从实现。前一条是**控制流基础**，后一条是**生态接入基础**。
2. **功能数量不是差距，功能"接线"是差距**：Nova 有 28 个工具、3 种 LLM 协议、4 个投影、完整的权限 AST，数量/深度上不弱；但 D-01、D-18 这类"实现存在却未接入/接入错"的问题集中在集成层，恰恰是 0 集成测试的直接后果。
3. **架构路线不同而非优劣**：Nova 是"事件溯源 + 薄壳 + 多协议"，codex 是"Item 生命周期 + app-server + 单协议"。前者更适合单机桌面与面试讲述，后者更适合多客户端/服务端治理。追平 codex 的 124 crate 拆分没有价值。

---

## 12. 需要重点关注与后续跟进

### 12.1 P0：立即修（影响可用性与面试问答）

| # | 动作 | 触及文件 | 验收标准 |
|---|---|---|---|
| P0-1 | **把 `collect_mcp_tools()` 接进 `get_available_tools_for_agent()`**，或改为在 `ToolRegistry::register_dynamic` 里注册 MCP 工具 | `N:crates/nova-core/src/agent/tools/mod.rs:738-790`、`N:.../capabilities/mcp_tools/mod.rs:78-102`、`N:.../tools/registry.rs:124` | 启动一个带工具的 MCP server（如 `rmcp_test_server`）后，模型工具清单里出现 `mcp__<server>__<tool>` 且带正确 `input_schema`；模型可直接调用而无需 `mcp_auth` |
| P0-2 | **补一条 MCP 端到端集成测试**（stub server + 握手 + tools/list + 注入 + 调用） | 新建 `crates/nova-core/tests/mcp_integration.rs` 或 `crates/nova-cli/tests/` | 测试能捕获"工具未注入"这类回归；`cargo test` 必须覆盖 `collect_mcp_tools` 的**调用点**而不只是函数本身 |
| P0-3 | **错误类型化最小集**：先定义 6 类 `TurnErrorKind{ContextWindowExceeded,RateLimitExceeded,Auth,Network,Model,Tool}`，让 `handle_turn_error` 按类分支 | `N:crates/nova-protocol/src/agent.rs`、`N:.../provider/stream_runner.rs:290-332`、`N:.../engine.rs:334-371,742-769`、`N:.../capabilities/compact/summary.rs:127-141` | 限流触发退避、超限触发压缩、鉴权失败给前端明确提示；删除 `is_prompt_too_long_error` 的字符串判断 |
| P0-4 | **流式重试状态机**：区分终态/可恢复，断流后从 `clone_history()` 重发，前端收到 `will_retry` 状态 | `N:.../provider/stream_runner.rs`、`N:crates/nova-protocol/src/agent.rs`（加 `TurnError.will_retry`） | 人为中断 SSE 后能自动恢复并继续同一 turn，不产生"整轮失败" |
| P0-5 | **面试链路口径**：把"输入如何提交 → 工具如何调度 → 结果如何落库"按 §2.3 Nova 时序讲清，并主动说明 D-01/D-03 的修复计划 | — | 能画出时序、指出 `session_events` append-only + 4 投影、说清 `begin_turn` 拒绝并发的原因 |

### 12.2 P1：本轮迭代修（影响健壮性与可维护性）

| # | 动作 | 触及文件 | 验收标准 |
|---|---|---|---|
| P1-1 | 修 D-18 四处接线缺陷：`ALWAYS_ON_TOOLS` 改 `"write_plan"` 并在 `is_tool_enabled` 里统一 `normalize_name`；`write_plan` 的 Err 走 `ToolFailure`；`spawn_agent` 真正使用 role 或从 schema 删掉；删除/接线 `compact_messages_for_turn` | `N:crates/nova-protocol/src/misc.rs:80-104`、`N:.../tools/write_plan_tool/write_plan.rs:86-97`、`N:.../tools/spawn_agent_tool/spawn_agent.rs:24-69`、`N:.../capabilities/compact/mod.rs:814,865` | 挂 bundle 时 `write_plan` 可见；失败工具结果 `is_error=true`；schema 与行为一致 |
| P1-2 | **MCP 协议升级**：至少支持 `2025-06-18`，读取并保存 `initialize` 返回的 capabilities，按能力决定是否发 `resources/*` | `N:.../mcp/stdio.rs:263-290`、`streamable_http.rs:359-387`、`N:crates/nova-protocol/src/mcp.rs` | 新版 server 能握手；无 capabilities 时不误发方法 |
| P1-3 | MCP `send_request` 改为后台读任务 + 分派器，不再丢弃非匹配 id 消息；补 `notifications/cancelled` 与取消 token | `N:.../mcp/stdio.rs:54-89`、`N:.../mcp/mod.rs:954-1011` | 用户点停止时 MCP 调用能取消；服务端通知不再导致挂起 |
| P1-4 | SSE transport：实现或**从配置枚举中移除**（避免用户配了却固定失败） | `N:.../mcp/sse.rs`、`N:crates/nova-protocol/src/mcp.rs:36-40`、`N:.../mcp/mod.rs:391-396` | 二选一，不允许"配置存在但运行时固定 Error" |
| P1-5 | 状态重放 RPC：把 `render_tool_logs`/`render_ui_history` 暴露成"按 `seq` 增量拉取"，或给事件加稳定 id 支持 upsert | `N:.../session/projection.rs:176,388`、`N:.../api/history.rs:73-144`、`N:crates/nova-protocol/src/agent.rs` | 模拟丢一条 `ToolCallCompleted` 后前端能自愈，不需整页重拉 |
| P1-6 | 契约校验升级：生成 JSON schema + 产物快照比对；给 `mcp.rs` 的类型加 `#[ts(export,...)]`；删除或同步 `bindings/protocol.ts` | `N:scripts/gen-bindings.js`、`check-contract.js`、`N:crates/nova-protocol/src/mcp.rs` | 改一个 Rust 字段后 `npm run check:contract` 必须失败 |
| P1-7 | 统一工具编排层：把审批/沙箱/拒绝重试收进一个 `execute_tool_with_app` 的泛型 run，让 apply_patch/Edit/Write 也能走沙箱 | `N:.../tools/mod.rs:822-881`、`N:.../sandbox/mod.rs`、`N:.../bash_tool/bash.rs:146-178` | 新增工具不必重写审批+沙箱；文件写工具也受 `WorkspaceWrite` 约束 |
| P1-8 | Windows 沙箱驱动接入 `active_driver()`，或明确文档说明 Windows 默认不隔离 | `N:.../sandbox/mod.rs:272-281`、`windows.rs` | 安全口径与实际行为一致 |
| P1-9 | 补 engine 端到端测试夹具（mock LLM SSE + 内存 SQLite），覆盖"工具调用→回灌→落库→投影" | 新建 `crates/nova-core/tests/` | 至少 1 条完整 turn 的集成测试 |

### 12.3 P2：有余力再做（收益递减）

| # | 动作 | 说明 |
|---|---|---|
| P2-1 | MCP OAuth / elicitation / 连接池 / templates / 大输出截断 / 状态码推送 | 依赖是否引入 `rmcp` SDK；若引入，可一次性拿到 §5 大部分能力 |
| P2-2 | `ToolSpec` 式统一工具定义 + namespace/freeform 形态 | 三协议适配下收益有限，优先级低于 P0/P1 |
| P2-3 | StepContext / 步级 timing / steer 队列 | 会动 `execute_turn` 614 行主体，风险大，除非要展示多客户端能力 |
| P2-4 | 多智能体 v2（异步 send_input/wait/interrupt）+ 持久化 graph/board | 目前单版本够面试讲述 |
| P2-5 | otel / metrics / turn timing | 项目无服务端运维需求 |
| P2-6 | 清理未使用的 anthropic adapter（若产品确定排除） | 可减约 1000 行维护面 |
| P2-7 | `mcp_auth` 改名（如 `mcp_gateway`） | 避免命名误导 |

### 12.4 明确不建议追

- codex 的 124 crate 拆分、Bazel 构建、guardian/attestation、realtime_webrtc、cloud-tasks、voice-host、code-mode 运行时、插件市场、企业合规与 EMA 服务端、2026-07-28 `server/discover` 发现流程——这些要么服务于 OpenAI 服务端治理，要么对"单机桌面编码助手 + 秋招面试展示"无增量。
- 上一版审计 §10 里"并行工具（`parallel`）"被列为不建议追，本次实读显示 Nova **已有**只读批并发（`tools/mod.rs:556-698`），无需再追。
- 上一版审计把 `ToolOrchestrator` 列为 P1（3–5 天），本次建议保留为 P1-7，但先做 P0-1～P0-4，因为接线与错误模型对用户可感知度更高。

---

## 13. 对上一版 `agent-architecture-audit.md` 的修正

| # | 上一版说法 | 本次实读结论 | 依据 |
|---|---|---|---|
| 1 | 基准"124 crates"、Nova"约 11k 行 Rust" | codex 确为 124 crate / **5927 个 `.rs`**；Nova 为 **226 个 `.rs` / 11985 行** | 实测统计 |
| 2 | §4 "所有工具（apply_patch、unified_exec、mcp…）走同一条路径"（`:312`） | `ToolOrchestrator` 只被 apply_patch 与 unified_exec runtime 调用；约 30 个控制类工具在 handler 内直接执行 | `C:.../handlers/apply_patch.rs:606-611`；各 handler 无 `ToolOrchestrator::run` 调用 |
| 3 | §4 "Nova 的 `ToolDisclosure::Deferred` + `LoadTool` 渐进披露是 Codex 没有的等价机制（Codex 用 `tool_search`）"（`:326`） | 表述自相矛盾：codex **有** `tool_search`，且是 Responses API 协议级 `ToolSpec::ToolSearch` + BM25 + namespace 归一，机制比 Nova 更完整 | `C:tools/src/tool_spec.rs:27-32`、`C:.../handlers/tool_search.rs:334-348`、`C:tools/src/tool_search.rs:98-132` |
| 4 | §3 "Nova 对应物：`Result<T,String>` 304 处，自定义错误枚举 2 个" | 数字准确，但需补充：codex 对外是 **`CodexErrorInfo` 22 变体**（上一版写 34 个变体且叫 `CodexErr`），内部是 `CodexErrorDetails` | `C:app-server-protocol/src/protocol/v2/shared.rs:73-131`（本次逐行验证） |
| 5 | §6 "全局 `HashMap<conversation_id, CancellationToken>`（**推断**）" | 已实读：`OnceLock<Mutex<HashMap<String, CancellationToken>>>`，`begin_turn` 占用、`finish_turn` 删除、`request_cancel` 支持兜底全部 | `N:.../cancellation.rs:5-10,25-45,47-65` |
| 6 | §5 "缺的不是投影，是把投影暴露成客户端可调用的重放接口" | 结论正确，但需补：投影层**已实现** `call_id` 终态配对/去重（`render_tool_logs`），且缺的是"增量重放"而非"全量重拉"（已有 3 个投影 RPC） | `N:.../session/projection.rs:388-446`、`N:.../api/history.rs:73-144` |
| 7 | §4 "并行执行：Nova 无 / Codex 有 `tools/parallel.rs`" | Nova **有**只读批 `JoinSet` 并发（≤10），只是没有 RwLock 读/写锁的 turn 级门禁 | `N:.../tools/mod.rs:556-698` |
| 8 | §9 "工具调度：各工具自管；Nova 加新工具要重写审批+沙箱+错误处理" | 前半不准：Nova 有**统一** `execute_tool_with_app` 做权限、`validate_tool_input` 做 schema、pre/post hook；真正分散的是"沙箱在 `shell_sessions`、提权重试在 `bash.rs`，且只有 Bash 有" | `N:.../tools/mod.rs:266-296,416-554,822-881` |

**上一版未发现、本次新发现的高价值项**：D-01（MCP 工具未注入清单）、D-18 四处接线缺陷、SSE stub、MCP 固定协议版本并丢弃 capabilities、`write_plan` 错误当成功、`ALWAYS_ON_TOOLS` 大小写不匹配、`compact_messages_for_turn` 休眠、`spawn_agent` 丢弃 role、Windows 沙箱驱动未被选用、无契约快照测试。

---

## 14. 证据边界与未验证项

以下明确标注，避免把"没读到"当成"没有"：

1. **Nova 侧未展开函数体**：`services/shell_sessions/mod.rs` 的 `run_foreground/run_background/kill_background_job/read_background_output/run_once_unsandboxed`、`services/model_catalog.rs` 正文、`services/settings.rs` 正文、`provider/client.rs::send_request` 正文、`provider/adapters/*/prompt.rs` 的 messages 转换前半、`nova_tools_to_anthropic_tools` 具体编码、`provider/token_usage_log.rs`、`sandbox/windows.rs` 后半、`hooks/command.rs:120-198`、`compact/mod.rs` 之外的 capabilities 正文、`api/*.rs` 除 mcp/history 外、`src/features/**` 大部分前端、`scripts/sync-cli.js`。这些模块的行为结论均来自**已实读的调用点**，未对内部实现下判断。
2. **codex 侧未展开函数体**：`core/src/session/turn.rs` 的 `try_run_sampling_request` 主体（`:2888-3827`）、`thread_manager/*.rs`、`compact_remote_v2*.rs`、`guardian/*`、`network_approval.rs`、`exec_policy.rs` 主体、`state/src/**`、`thread-store/src/**`、`app-server/src/**` 路由主体、`rmcp-client` 的 OAuth/EMA 主体、`tools/src/json_schema/*`。四个子代理报告均附有完整"未实读"清单。
3. **`[推]` 标记的两处**：D-01 的成因（渐进披露重构后注入环节漏接）未做 git 历史考证；`WebSearch` 引擎稳定性判断基于"HTML 爬虫解析"这一实读事实，未做线上验证。
4. **行号时效**：codex 基准是 2026-11-08 的 commit `748ab55`，Nova 是本次会话工作区状态；任一仓库更新后行号会偏移，结论需重新核对。
5. **未做动态验证**：本次是静态代码比对，没有运行 `cargo test`、没有启动 MCP stub server、没有抓实际 SSE 流。D-01 的结论来自"全仓 grep 无调用点 + 三个 adapter 只取内置注册表"的静态证据，建议用 P0-2 的集成测试做动态确认。

---

## 15. 快速索引

| 想查什么 | 章节 |
|---|---|
| 比对范围与维度怎么定的 | §0 |
| 十条主要差异 / 一句话总评 | §1 |
| 端到端数据流（两条时序） | §2.3 |
| Agent 内核逐机制 | §3 |
| 28 个工具逐个对照 | §4.2 |
| codex 独有的工具族 | §4.3 |
| MCP 逐能力矩阵 | §5.6、§5.7 |
| MCP 工具为什么调不到 | §5.0（D-01） |
| LLM 三种请求体 / tool-calling 编码 | §6.1–§6.2 |
| SSE 事件清单 / 重试状态机 | §6.3–§6.4 |
| 客户端协议与 Item 模型 | §6.5–§6.7 |
| 接口 / 依赖 / 错误 / 测试 | §7–§10 |
| 差异结论矩阵与归纳 | §11 |
| 重点关注与跟进清单 | §12 |
| 对上一版审计的修正 | §13 |
| **多智能体专项深挖（第二轮）** | **§16** |

---

## 16. 多智能体专项比对（第二轮逐项实读）

> **为什么单独重看**：§3.8 与 §4.2.5 只覆盖了 `orchestration/` 四个文件和三个工具，结论停留在"单版本 + 进程内黑板 + 无持久化"。第二轮把两边多智能体相关代码全部展开，发现若干**结构级**问题（重复工具入口、角色参数被丢弃、身份三套来源、休眠的 board 能力、取消不可单独生效），并纠正了 codex 侧 4 处命名误判。本章覆盖并修正 §3.8 / §4.2.5 / D-19 / §12 P2-4 的判断。
> **规模实测**：Nova 多智能体相关 Rust **1499 行 / 11 个文件**（`orchestration/*` + 三个工具 + `api/collaboration.rs`）；codex 相关 **118 个 `.rs` / 29418 行**（`core/src/agent/` 含 `control/`+`host/`、v1/v2 handlers、`agent-roles/`、`agent-graph-store/`、四个 message-board crate 等）。约 20 倍差距，且差距不在"工具数量"而在**生命周期与通信基础设施**。

### 16.0 概念校准：两边"智能体"指的不是同一件事

| 概念 | Nova | codex |
|---|---|---|
| 会话级人格配置 | `AgentBundle`（`N:crates/nova-protocol/src/misc.rs:41-76`）：`prompt` + `enabled_tools`/`enabled_skills`/`enabled_mcp_servers` 清单，挂在 `conversations.active_agent_id`，**按会话生效**（`agent_bundles/mod.rs:5-12,92-100`） | 无直接对等物；`agent-roles` 是**子代理角色**，不是会话配置 |
| 派生子代理 | `subagent::run`（`N:.../orchestration/subagent.rs:133-247`），ID `{parent}:sub:{uuid}` | `ThreadSpawn`（`C:protocol/src/protocol.rs:3250-3268`），`AgentPath` + `AgentMetadata{depth, agent_path, agent_nickname, agent_role}`（`C:core/src/agent/types.rs:11-19`） |
| 版本切换 | **无**。所有 28 个内置工具恒在清单里（除 bundle/披露/子代理过滤） | `MultiAgentVersion::{Disabled,V1,V2}`（`C:protocol/src/protocol.rs:3428-3432`），由 `spec_plan.rs:767-819 collab_tools_enabled` 决定暴露哪一套 |
| 分支 | `branch.rs`：纯问答、**不落库、无工具**，前端内存态全量上传历史（`N:.../orchestration/branch.rs:1-9,80-129`） | `fork_thread` / `ForkPersistence`：**保留历史的真分支**（`C:core/src/thread_manager.rs:1801,1819-1822`） |
| 角色 | **不存在子代理角色系统** | `agent-roles/` + `core/src/agent/role.rs`：内置 `default/explorer/worker`，只削弱不放权 |

**codex 侧 4 处必须纠正的命名误判**（本次全局检索 + 实读确认，避免按 crate 名望文生义）：

1. `core/src/tools/multi_agent_tool.rs` **不是 legacy 兼容层**，是 V2 catalog 的 spec 覆盖/命名空间包装器：`multi_agent_v2_handler(..., namespace, description_override, parameters_override)` 把 `ToolSpec::Function` 重新包成 `ToolSpec::Namespace`（`:19-20,69-97`），默认 namespace 为 `"collaboration"`（`C:core/src/config/mod.rs:275`）。
2. **`AgentRole` / `AgentType` / `AgentIdentity` 三个类型在 `codex-rs/` 全仓不存在**（`enum AgentRole|struct AgentIdentity|...` 检索 `No matches found`）。实际类型是 `AgentStatus`（`C:protocol/src/protocol.rs:2050-2070`）、`SubAgentSource`、`AgentMetadata`、`AgentRoleConfig`（`C:agent-roles/src/agent_role_config.rs:9-18`，是结构体不是枚举）。
3. `agent-identity/` **与子代理拓扑无关**：是 ChatGPT 后端 agent 身份认证（Ed25519 / JWT，`AGENT_IDENTITY_JWT_AUDIENCE="codex-app-server"`，`agent-identity/src/lib.rs:38-46,115-129`）；`runtime-agent-instance/` 同理是云端 runtime 注册。
4. `collaboration-mode-templates/` **与多智能体无关**：只有 `PLAN` / `DEFAULT` 两个 developer-instructions 模板，由 `<collaboration_mode>` 标记切换 Plan 模式（`templates/default.md:1-5`）。`ext/agent/`（legacy `AgentRunner`）、`ext/items/`、`ext/goal/`、`message-board-mcp/`（board 的 MCP 传输层）也不是 v1/v2 工具族实现主体。

### 16.1 工具族逐项对照

**Nova：5 个工具，但只有 1 种子代理**（关键发现）：

| 工具 | 定义 | 参数 | registration | 实际调用 |
|---|---|---|---|---|
| `Task` | `task_tool/task.rs:14-48` | `task`(必), `description`(3-5 词摘要) | `read_only=true, Core`（`:8-11`） | `subagent::run`（`:101`） |
| `spawn_agent` | `spawn_agent_tool/spawn_agent.rs:19-43` | `task`(必), `role`, `nickname` | `read_only=false, Core`（`:9-16`） | **同一个** `subagent::run`（`:67`） |
| `board_post` | `board_tool/board.rs:36-60` | `channel`,`content`(必),`thread_id` | `read_only=false, Core` | `AgentMessageBoard::post_message` |
| `board_read` | `:62-85` | `channel`,`thread_id`,`limit`(默认 20) | `read_only=true, Core` | `list_posts` |
| `board_list_channels` | `:87-97` | 无 | `read_only=true, Core` | `list_channels` |

**D-21（重复工具入口 + 并发语义相反）**
- **表现**：`Task` 与 `spawn_agent` 都直接调 `crates/nova-core/src/agent/orchestration/subagent.rs:133` 的 `run(core, parent, task)`，返回同一个 `report`；差别只有 `Task` 多跑 `SubagentStart/Stop` hooks 并把 `description` 当 hook 名（`task.rs:82-88,91-111`），`spawn_agent` 声明了 `role`/`nickname` 却在 `:57-65` 赋给 `_role`/`_nickname` **直接丢弃**。更关键的是 `Task` 标 `read_only=true`（`task.rs:11`，注释写明"进入批量并发执行器后同一轮的多个 Task 可并行运行"），`spawn_agent` 标 `read_only=false`（`spawn_agent.rs:13`）——后者在 `execute_tool_calls_with_app` 里会触发先 flush 只读批再串行执行（`tools/mod.rs:556-698`）。
- **原因**：推测是先后加了两套入口（Claude 风格的 `Task` 与"多智能体"命名的 `spawn_agent`），未做合并；两个 description 还给了模型不同的使用暗示（`Task` 强调"独立研究子任务、可并行"，`spawn_agent` 强调"dedicated role / registered into multi-agent graph"）。
- **影响**：① 模型看到两个语义重复的工具，选择哪个**直接决定本轮能否并行**；② 走 `spawn_agent` 时描述里承诺的 "dedicated role" 完全不兑现，graph 里 `role` 恒为 `"researcher"`、`nickname` 恒为 `"ResearchSubagent"`（`subagent.rs:148-149`）；③ 两个工具都返回阻塞式报告，模型无法"派出去后继续干别的"。
- **证据**：`N:.../tools/task_tool/task.rs:8-11,51-59,101-103`；`N:.../tools/spawn_agent_tool/spawn_agent.rs:9-16,19-43,51-69`；`N:.../orchestration/subagent.rs:133-137,145-155`；`N:.../tools/mod.rs:556-698`。

**codex：两套工具族，按版本只暴露一套**

| 版本 | 形态 | 工具 | 参数要点 | 返回 |
|---|---|---|---|---|
| V1 | `ToolSpec::Namespace("multi_agent_v1")` | `spawn_agent` / `send_input` / `resume_agent` / `wait_agent` / `close_agent` | spawn: `message,items,agent_type,fork_context,model,reasoning_effort`（required=**None**）；wait: `targets[],timeout_ms` | `{agent_id,nickname}` / `{status,timed_out}` / `{previous_status}` |
| V2 | 6 个独立 `ToolSpec::Function` | `spawn_agent` / `send_message` / `followup_task` / `wait_agent` / `list_agents` / `interrupt_agent` | spawn: `task_name,message`(必), `agent_type,fork_turns("all"\|"none"),model,reasoning_effort`；`fork_context` 明确报错 | `{task_name,nickname?}` / wait 返回 `{message,timed_out}` 摘要 / list 返回 `{agents:[{agent_name,agent_status}]}` |

证据：`C:core/src/tools/handlers/multi_agents_spec.rs:14,70-106,108-164,166-202,204-234,236-263,265-285,287-301,303-313,315-334,336-359,361-382,384-431,479-568,610-684,889-927`；`C:core/src/tools/handlers/multi_agents.rs:74-84`；`C:core/src/tools/handlers/multi_agents_v2.rs:28-42`；`C:core/src/tools/spec_plan.rs:767-819`。

- **V1 → V2 的实质变化**：寻址从 `ThreadId` 字符串变 `AgentPath`（`agent_resolver.rs:9-30`）；`fork_context:bool` 变 `fork_turns:"all"|"none"`；`send_input` 拆成 `send_message`(QueueOnly，不触发 turn) 与 `followup_task`(TriggerTurn，idle 时启动 turn)（`multi_agents_v2/{send_message,followup_task}.rs:41-48`）；`wait_agent` 从"等具体 agent 到 final status"变"等任意 mailbox 更新 / steer / 超时"（`multi_agents_v2/wait.rs:52-110`）；新增 `list_agents`/`interrupt_agent`，移除 `resume_agent`/`close_agent`。
- **影响**：codex 模型可以先 `spawn_agent` 拿 id 继续做别的事，再 `send_message`/`followup_task`/`wait_agent`/`list_agents`/`interrupt_agent`；Nova 只有"调用即阻塞等报告"一种模式。

### 16.2 生命周期状态机

| | Nova | codex |
|---|---|---|
| 状态类型 | 只有图边 2 态 `ThreadSpawnEdgeStatus{Open,Closed}`（`N:.../graph.rs:10-18`）；运行态靠**字符串** `phase: "start"|"done"|"error"`（`subagent.rs:162,239`） | `AgentStatus` **7 变体**：`PendingInit/Running/Interrupted/Completed(Option<String>)/Errored(String)/Shutdown/NotFound`（`C:protocol/src/protocol.rs:2050-2070`） |
| 转换 | `upsert_node(Open)`（`:156`）→ 结束 `set_node_status(Closed)`（`:206-210`），**无中间态、无失败态区分**（error 与 done 都置 Closed） | 唯一转换点 `agent_status_from_event`（`C:core/src/agent/status.rs:6-31`）：`TurnStarted→Running`、`TurnComplete{error}→Errored|Completed(last_agent_message)`、`TurnAborted→Interrupted/Errored`、`ShutdownComplete→Shutdown`；`is_final()` 排除 `PendingInit/Running/Interrupted` |
| 索引 | `HashMap<conversation_id, HashMap<node_id, AgentNode>>`（`graph.rs:37-40`） | `AgentRegistry{agent_tree,thread_paths,used_agent_nicknames,nickname_reset_count}` + `AtomicUsize`（`C:core/src/agent/registry.rs:28-40`），按 path/id/nickname 三向索引与去重 |
| 控制接口 | **无**。`subagent::run` 是唯一函数，无 `AgentControl` 类 trait | `AgentControl` trait **20 个方法**（`C:core/src/agent/api.rs:41-161`）：`resolve/spawn/send/take_mailbox/watch_mailbox/ensure_child_loaded/interrupt/list/child_agent_paths/check_turn_admission/admit_turn/record_usage/turn_finished/service_tier/propagate_config_update/get_guardian_package/pending_budget_reminder/mark_budget_reminder_delivered/identity` |

**D-22（无状态机 ⇒ 无异步协作）**
- **表现**：Nova 的"子代理"是一个 `async fn run()` 的调用栈，没有可寻址的实体、没有状态查询、没有 send/wait/resume/close/interrupt。`Task`/`spawn_agent` 返回时子代理已经结束且事件被删（`subagent.rs:187`）。
- **原因**：`subagent::run` 复用 `AgentEngine::execute_turn`（`:181-182`），即"把一次完整 turn 当作子代理"，没有引入独立 thread/registry 抽象。
- **影响**：① 无法"派一个 agent，几分钟后追问"；② 无法查询"哪些 agent 还活着"；③ 无法中断单个子代理；④ 失败与成功只是 `Result<String,String>` 与一个 `phase` 字符串，前端拿不到 `Errored(reason)` 之外的结构化信息。
- **证据**：`N:.../orchestration/subagent.rs:133-247`、`N:.../graph.rs:10-33,63-79`；`C:protocol/src/protocol.rs:2050-2070`、`C:core/src/agent/status.rs:6-31`、`C:core/src/agent/api.rs:41-161`。

### 16.3 派生：历史、配置继承、身份与角色

| 项 | Nova | codex |
|---|---|---|
| 初始历史 | `execute_turn(&sub_id, task, None, ...)`（`subagent.rs:182`）：**只传 task 一条消息，不继承父历史**；子代理系统提示词明确写 "Each task is independent: do not rely on any context outside this conversation."（`:78`） | `prepare_forked_history`（`C:core/src/agent/history.rs:161-483`）：flush rollout → `load_agent_model_context` → `keep_forked_rollout_item` 过滤 → 追加 `MultiAgentUsageHint` → `InitialHistory::Forked`（`:479-482`）；V2 `fork_turns="all"` 默认全量继承 |
| 子事件持久化 | 用完即删：`delete_events(core,&sub_id)`（`subagent.rs:186-187`）；graph/board 也只在内存 | rollout + state DB 持久化；`ForkPersistence::{Copied,CopiedDeferred,Referenced}` 三模式（`C:core/src/session/mod.rs:470-479,1872-1917`），`Referenced` 只记 `history_base + inherited_item_count` 不复制祖先记录 |
| 配置继承 | **隐式全量共享**：同一 `Core`、同一全局 settings、同一 model/provider、同一 cwd；子代理只是 `is_subagent_conversation()` 触发的几处 `if` 分支（工具白名单/`system_prompt`/权限 Deny/事件分流/`max_tokens`/`reasoning`） | `prepare_agent_spawn_config`（`C:core/src/agent/child_config.rs:53-113`）显式构造子 `Config`：从父 `turn.config` 克隆，覆盖 model/provider/cwd/approval_policy/permission_profile/approvals_reviewer/developer_instructions/exec_policy/environments，`service_tier` 按是否共享决定；model 与 reasoning_effort 可被 spawn 参数覆盖并校验（`:346-393`） |
| 角色 | **无**。`spawn_agent` 的 `role`/`nickname` 丢弃（`spawn_agent.rs:57-65`）；graph 硬编码 `role:"researcher"`, `nickname:"ResearchSubagent"`（`subagent.rs:148-149`） | `AgentRoleConfig{description,config_file,nickname_candidates}`（`C:agent-roles/src/agent_role_config.rs:9-18`）；`role.rs:47-125,171-240` 用配置层覆盖 model/developer_instructions/effort/personality/skills，**只允许禁用** ShellTool/Apps/Plugins/MemoryTool/RequestPermissionsTool（文件头："never replace the parent session's authority"）；内置 `default/explorer/worker`（`:315-391`），角色描述注入 `spawn_agent` 工具文本（`:242-306`） |
| `AgentBundle` 与子代理 | **不相交**。`system_prompt.rs:128-138` 的子代理早退在 `active_bundle()` 调用（`:140`）之前；`tools/mod.rs:745` 的子代理白名单早退在 bundle 过滤（`:758`）之前。子代理 ID 形如 `{parent}:sub:{uuid}`，也不会命中 `CONVERSATION_AGENT_CACHE` | 角色作用于子线程，配置层是显式的 spawn 参数 |

**D-23（角色是空壳，且 bundle 与子代理是两套不相交的"智能体"）**
- **表现**：`spawn_agent` 的 schema 声明 `role`（"researcher/reviewer/architect/debugger"）与 `nickname`，执行时两者被读进 `_role`/`_nickname` 后丢弃；graph 节点、board 作者、前端状态里全用硬编码身份。同时项目另有一套 `AgentBundle`（会话级人格配置，`api/agent_config.rs` 有 12 个 RPC 方法），与子代理毫无关联。
- **原因**：`spawn_agent` 像是按"多智能体"设想加的壳，底层 `subagent::run` 只接受 `(core, parent, task)` 三参数，没有 role 通道；`AgentBundle` 是另一条产品线（智能体配置页）。
- **影响**：① 模型传 `role:"reviewer"` 不会改变任何行为（提示词、工具集、model 都不变）；② 三个身份来源互不一致——graph 用 `researcher/ResearchSubagent`（`subagent.rs:148-149`）、`board_post` 工具用 `subagent/Subagent`（`board_tool/board.rs:123-127`）、用户经 `api/collaboration.rs:48-49` 发帖用 `user/User`；③ 面试若讲"多智能体有角色分工"会被追问穿。
- **证据**：`N:.../tools/spawn_agent_tool/spawn_agent.rs:22-41,57-69`；`N:.../orchestration/subagent.rs:145-155`；`N:.../tools/board_tool/board.rs:123-127`；`N:.../api/collaboration.rs:46-59`；`N:.../utils/system_prompt.rs:121-140`；`N:crates/nova-protocol/src/misc.rs:41-76`；`C:core/src/agent/role.rs:1-4,47-125,171-240,315-391`。

**D-24（不 fork 历史，也不保留子会话）**
- 表现：子代理从零上下文起步，结束即 `delete_events`；`AgentGraphUpdated`/`SubagentStatus`/`BoardPostCreated` 只走 `core.emit` 不落 `session_events`。
- 原因：把"派生子代理"实现成"隔离一次 turn"，避免污染父上下文（这是**有意的设计取舍**，见 `Task` description: "intermediate steps do NOT consume this conversation's context"）。
- 影响：① 子代理看不到父对话已有结论，`task` 必须完全自包含；② 重启或重开面板后，子代理运行记录、图谱、看板全部消失；③ 无法从历史中间分叉出新会话。codex 的 `ForkPersistence` 与 cold resume（`spawn.rs:112-170 restore_v2_agent_metadata`）都做不到。
- 证据：`N:.../orchestration/subagent.rs:180-187`、`N:.../tools/task_tool/task.rs:17,26-27`；`C:core/src/agent/history.rs:161-483`、`C:core/src/session/mod.rs:470-479,1872-1917`。

### 16.4 并发、深度、取消

| 项 | Nova | codex |
|---|---|---|
| 并发上限 | `static LIMIT: OnceLock<Semaphore> = Semaphore::new(3)`（`subagent.rs:20,36-39`），**进程级全局**，不区分父会话 | V1 `DEFAULT_AGENT_MAX_THREADS=Some(6)`；V2 `max_concurrent_threads_per_session=4` → `effective = 4-1 = 3`（`C:core/src/config/mod.rs:270-274,1759-1773`），另有执行级 `AgentExecutionGuard`（`control/execution.rs:34-88`） |
| 并行触发 | 只有 `Task` 标 `read_only=true` 能进 `JoinSet` 只读批（`NOVA_MAX_TOOL_USE_CONCURRENCY` 默认 10）；`spawn_agent` 串行 | `ToolRouter` 的 `supports_parallel_tool_calls` + `RwLock` 读/写锁 |
| 深度/递归 | **无计数**。靠 `SUBAGENT_ALLOWED_TOOLS` 8 项白名单排除 `Task`/`spawn_agent`/`mcp_auth`/写工具（`subagent.rs:24-34`）；`is_subagent_conversation` 只判 ID 含 `:sub:`，嵌套子代理的 ID 会变成 `{parent}:sub:{uuid}:sub:{uuid}` 仍返回 true，不会被额外拦截 | V1 `next_thread_spawn_depth` + `DEFAULT_AGENT_MAX_DEPTH=1`（`C:core/src/agent/registry.rs:84-98`、`config/mod.rs:280,1006-1007`），handler 里超限直接回 "Agent depth limit reached. Solve the task yourself."；**V2 无深度限制**（注释 "Ignored by V2"），靠容量 + `spec_plan.rs:774-778` 的 model/path 门控 |
| 取消 | `subagent::run:164-184` 取父 token，new 一个 `sub_cancel`，`tokio::spawn` 把父 `cancelled()` 转发过去；**全程不调 `cancellation::begin_turn`**（全仓 `begin_turn` 仅 3 处：`agent/mod.rs:24` 主 turn、`branch.rs:105` 分支、定义处） | `child_token()` 级联（`C:core/src/codex_delegate.rs:206-211,251,319`）+ 全树单一 `shutdown.cancel()`（`agent/control/runtime.rs:316-320`）+ spawn 中途 `PendingSpawn::drop` 清理（shutdown 子 thread、丢弃持久化、把 spawn 边置 `Closed`，`spawn_guard.rs:71-124`）+ `interrupt_agent` 只发 `Op::Interrupt` 不回收 |
| 资源回收 | 无 `close` 概念；Semaphore permit 由 RAII 自动释放，graph 边置 `Closed`，子事件删除 | `close_agent` 递归 `shutdown_agent_tree`（BFS 后代）→ 写 `Closed` 边 → `release_spawned_thread` 释放并发计数 → `mailboxes.remove`（`C:core/src/agent/control/legacy.rs:50-127`） |

**D-27（取消只能整父级，单个子代理不可寻址）**
- 表现：`request_cancel(Some(sub_id))` 查不到 token（子代理从未 `begin_turn` 注册），唯一能取消子代理的方式是取消整个父会话；也没有"中断某个子代理但保留父 turn"的路径。
- 原因：取消注册表是 `HashMap<conversation_id, CancellationToken>`，而子代理的 token 是 `run()` 内部新建的局部变量，只与父 token 单向联动。
- 影响：① 3 个子代理并行时无法只停掉跑偏的那一个；② 用户点停止 → 整轮终止；③ 无 spawn 中途取消的清理（若 `Semaphore::acquire` 期间父被取消，`run` 已经建了 graph 节点但还没注册取消转发，`AgentGraphUpdated` 会残留一个永远 Open 的节点）。
- 证据：`N:.../orchestration/subagent.rs:138-142,156-160,164-184`、`N:crates/nova-core/src/agent/cancellation.rs:25-45`（全仓 `begin_turn` grep 结果）；`C:core/src/codex_delegate.rs:206-211`、`C:core/src/agent/control/runtime.rs:316-320`、`C:core/src/agent/control/spawn_guard.rs:71-124`。

### 16.5 通信：board vs mailbox / InterAgentCommunication / message-board

**Nova 的 board（`N:.../orchestration/board.rs`，321 行）**：进程内 `Arc<RwLock<HashMap<root_conversation_id, ConversationBoard>>>`（`:85-88`），每会话默认 3 频道 `general/research/evidence`（`:42-81`），`BoardPost{id,channel,thread_id,author_id,author_nickname,author_role,content,created_at}`（`:22-33`）；API 有 `create_channel/list_channels/post_message/list_posts/search_posts`（`:120-271`）。

**codex 的三条通道**（实读）：
1. **mailbox**：`Mailboxes(Mutex<Option<HashMap<ThreadId, Mailbox>>>)` 的 `enqueue/take/watch/remove`（`C:core/src/agent/control/mailbox.rs:15-116`），`watch::Receiver<bool>` 用于唤醒。
2. **InterAgentCommunication**：`delivery.rs:11-45` 把消息转 `InterAgentCommunication{author,recipient,content,encrypted_content,trigger_turn}`，`context/inter_agent_message.rs:47-72` 渲染成 `Message Type/Task name/Sender/Payload` 的 **上下文 fragment**；`session/handlers.rs:83-101` 在 `trigger_turn || WakeOnAgentMessages` 时 `maybe_start_turn_for_pending_work` 唤醒父代理。
3. **message board 分层子系统**：`message-board/`（共享 `AgentMessageBoard` trait + Local/InMemory/Remote 后端，`api.rs:42-131` 有 `create_channel/upsert_channel/list_channels/post/append/list_threads/search_posts/read_thread/read_post/set_subscription/subscribed_channels`）← `ext/agent-message-board/`（构造后端+注册工具）← `agent-message-board-host/`（`install_agent_message_board` 按 feature 选 5 种 `BoardBackend::{Mcp,Connected,Remote,InMemory,Local}`，`lib.rs:44-107`）← `agent-message-board-client/`（remote 传输 + `TurnLifecycleContributor` 通知，`:30-70`）；`message-board-mcp/` 只是 `BoardBackend::Mcp` 时把工具挂到 rmcp 的适配层，**不拥有存储/通知/生命周期**。

**D-25（board 是 UI 展示级，不进模型上下文）**
- **表现**：Nova 子代理成功后 `subagent.rs:218-233` 自动把完整报告 `post_message(..., "research", ...)` 并 emit `BoardPostCreated`；但这是**前端事件**，不会注入父模型上下文，父代理只拿到 `ToolOutcome` 里的那份 report。board 里唯一能进模型上下文的路径是子代理主动调 `board_read`（`board_tool/board.rs:151-172`）——只有当多个 `Task` 在同一轮并行、且后跑的子代理主动去读时才有意义。
- **原因**：board 被设计成"给人看的协同看板"（`board.rs:1-4` 自称"共享黑板"），事件通道 `board-post-created` 直接给 UI；codex 把 agent 间消息设计成模型输入 fragment。
- **影响**：① 两个并行子代理无法自动互知结论（除非各自主动 `board_read`，且阻塞式执行下时序也不保证）；② 子代理完成时不会唤醒父代理产生新 turn；③ `trigger_turn` 语义完全缺失，所以"派出去 → 收到通知 → 继续推理"这条链在 Nova 里不存在。
- **证据**：`N:.../orchestration/subagent.rs:218-233`、`N:.../tools/board_tool/board.rs:151-190`、`N:.../events.rs:172-183`；`C:core/src/agent/control/{mailbox.rs:15-116,delivery.rs:11-45,completion.rs:27-155}`、`C:core/src/context/inter_agent_message.rs:47-72`、`C:core/src/session/handlers.rs:83-101`。

**D-28（board 有三处休眠能力与两处实现瑕疵）**
- **表现**：
  - `create_channel`（`board.rs:120-148`）与 `search_posts`（`:246-271`）**全仓无调用点**（只有单测调 `search_posts`）；工具层只暴露 post/read/list_channels 3 个，RPC 层只有 `get_agent_graph/get_board_channels/get_board_posts/post_board_message` 4 个（`dispatch.rs:152-155`）。`NovaEvent::BoardChannelCreated`（`events.rs:73,186`）也从未被 emit。
  - `BoardPost.thread_id` 有字段、`board_post` 工具能传、`board_read` 能按 thread 过滤，但**没有创建 thread 的工具/RPC**，thread_id 只能从已有 post 的 `id` 手抄，主题讨论实际不可用。
  - `with_board_read`（`board.rs:108-117`）用的是 `self.boards.write().unwrap()`——**读操作也拿写锁**，多个 `board_read` 并发时串行化。
  - `graph.rs:146-162` 的 `list_descendants_bfs` 在 `status_filter=Some(s)` 时把 `queue.push_back` 放在 `if status_ok` 内，导致**被过滤节点的后代一并被剪枝**（例如过滤 `Open` 时，Closed 节点下的 Open 孙子不会被遍历）。
- **原因**：board/graph 按"最小可用"实现后未补齐入口；与 §5 的 D-01 属同类"实现存在、接线缺失"。
- **影响**：自定义频道、全文搜索、主题回复、订阅这些能力对用户与模型都不可达；BFS 剪枝会让 `AgentGraphUpdated` 的图谱视图在特定过滤条件下漏节点。
- **证据**：`N:.../orchestration/board.rs:108-117,120-148,246-271`、`N:.../orchestration/graph.rs:130-166`、`N:.../events.rs:73,186`、`N:.../api/collaboration.rs:6-67`、`N:crates/nova-cli/src/serve/dispatch.rs:152-155`（全仓 grep `create_channel|search_posts|BoardChannelCreated` 仅命中定义/测试/事件映射）。

### 16.6 拓扑与持久化

| 项 | Nova | codex |
|---|---|---|
| 存储 | `OnceLock<AgentGraphStore>` + `Arc<RwLock<HashMap<String, HashMap<String, AgentNode>>>>`（`graph.rs:37-47,180-184`），**进程内** | `LocalAgentGraphStore{state_db: Arc<StateRuntime>}`（`C:agent-graph-store/src/local.rs:10-29`），**SQLite 持久化** |
| 接口 | `upsert_node/set_node_status/get_node/list_children/list_descendants_bfs/get_all_nodes`（`graph.rs:50-177`），同步、无 trait | `AgentGraphStore` trait 4 方法（`C:agent-graph-store/src/store.rs:13-62`）：`upsert_thread_spawn_edge / set_thread_spawn_edge_status / list_thread_spawn_children / list_thread_spawn_descendants`，带 `status_filter` BFS |
| 写入时机 | `subagent::run` 开头 upsert `Open`（`:156`）、结尾 `set_node_status(Closed)`（`:206-210`） | spawn 时 `persist_thread_spawn_edge_for_source(..., Open)`（`C:core/src/agent/control.rs:633-664`）；close 写 `Closed`（`legacy.rs:64-84`）；取消由 `spawn_guard` 兜底 |
| 恢复 | **无**。重启后 graph/board/子代理面板全空，`api/collaboration.rs:6-12` 读的也是内存 | cold resume：`restore_v2_agent_metadata` 用 `list_thread_spawn_descendants(root, Some(Open))` 恢复身份（`C:core/src/agent/control/spawn.rs:112-170`），有 `multi_agent_resume/restore` 测试 |
| 失败态 | error 也置 `Closed`，`result_preview=None`（`subagent.rs:205-210`），成功/失败在图里不可区分 | `AgentStatus::Errored(reason)` 独立态；完成通知按 `error_info` 选模板（`completion.rs:92-105`） |

**D-26（进程内状态 + 事件一次性推送）**
- **表现**：graph/board 是 `OnceLock` 单例内存；四个协同 RPC 只是读内存；子事件 `delete_events` 删除；`SubagentStatus`/`AgentGraphUpdated`/`BoardPostCreated` 只经 `core.emit` 推前端（`subagents.ts` 用模块级 `reactive(new Map())`，`clearAllSubagents()` 清空）。
- **原因**：桌面单进程假设 + "子代理用完即弃"的取舍（§16.3）。
- **影响**：① 重启后协同数据全丢；② 前端丢一条 `subagent-event` 就永久缺条目，`get_agent_graph` 读同一份内存、无法自愈；③ 与 §6.5 的 D-07 同根：事件流而非 Item 集合，在多智能体面的投影。
- **证据**：`N:.../orchestration/{board.rs:274-278,graph.rs:180-184,subagent.rs:186-187}`、`N:.../events.rs:118-183`、`N:src/features/chat/services/subagents.ts:41,119-178`；`C:agent-graph-store/src/{store.rs:13-62,local.rs:10-29}`。

### 16.7 事件与 UI 协议

| | Nova | codex |
|---|---|---|
| 通道 | **5 个专用 Tauri 事件**：`subagent-event`、`agent-graph-updated`、`board-post-created`、`board-channel-created`、`branch-event`（`events.rs:82-194`） | 统一 app-server v2 `ThreadItem`：`CollabAgentToolCall{id,tool,status,sender_thread_id,receiver_thread_ids,prompt,model,reasoning_effort,agents_states}` + `SubAgentActivity{id,kind,agent_thread_id,agent_path}`（`C:.../v2/item.rs:481-513,1073-1099,1345-1442`） |
| 来源 | `subagent::run` 直接 `core.emit`（`subagent.rs:99-116,157-160,212-232,235-244`）；流增量由 `stream_runner.rs:38-44` 按 `:sub:` 分流成 `SubagentStream` | handler 发 `TurnItem::CollabAgentToolCall` started/completed；legacy `Collab*Begin/End` 是其反向投影（`C:protocol/src/legacy_events.rs:286-341`）；V2 的 send_message/followup_task/interrupt_agent/list_agents **不发 legacy UI 事件**（`:290-294`） |
| 状态语义 | `phase: String`（`start/done/error`）+ `elapsed_ms`/`report_preview`/`detail`；前端按字符串映射（`subagents.ts:120-145`） | 类型化 `CollabAgentToolCallStatus` + `SubAgentActivityKind{Started,Interacted,Interrupted,Completed}` |
| 流隔离 | `stream_runner.rs:30-44` 按前缀改道；`engine.rs:702` 跳过子代理的会话活跃度刷新；`engine.rs:440-455` 把 token 用量归并到父会话（`source="subagent"`） | agent 有独立 thread/turn，事件走各自 sub_id，用量走 `AgentControl::record_usage` |
| 可重放 | 无 | Item 生命周期 + `thread_history_projection`，可按 id 重放 |

**值得肯定的一处**：Nova 的子代理**用量归并**做得比概览里看起来细——`engine.rs:440-455` 把 `conversation_id` 换成父 ID 并打 `source="subagent"` 落 `token_usage_log`；流式日志也有独立 `subagent-event` 通道。这是多智能体可观测里少数做到位的地方。

### 16.8 错误、hooks 与测试

| 项 | Nova | codex |
|---|---|---|
| 失败表达 | `Result<String,String>`；`Task`/`spawn_agent` 用 `ToolFailure::new`（`task.rs:103`、`spawn_agent.rs:69`）；空报告 → `"subagent finished without producing a report"`，取消 → `"[subagent cancelled by user]"`（`subagent.rs:189-203`） | `AgentTurnOutcome{status,error_info:Option<CodexErrorInfo>}`；`AgentLimitReached{max_threads}`（`C:protocol/src/error.rs:134-135`）；`AgentErrorContext` 10 变体（`agent_error.rs:9-20`：`NicknameUnavailable`/`DuplicatePath`/`ForkHistory`/`ChildStartup`…） |
| hooks | 2 个事件 `SubagentStart/Stop`（`hooks/types.rs:26-27`），**只有 `Task` 触发**（`task.rs:91-111`），`spawn_agent` 完全不触发；可 `override_error`、注入 `additional_messages`、`prevent_continuation` | 14 事件 + `TurnLifecycleContributor`；`subagent_notifications.rs` suite |
| 容量错误 | 无。`Semaphore::acquire` 超限只是**排队**（`subagent.rs:138-141`）；UUID 不会冲突 | `reserve_spawn_slot→AgentLimitReached`；`reserve_agent_path→DuplicatePath`；`reserve_agent_nickname` 候选耗尽加 ordinal 后缀（`registry.rs:267-325`） |
| 测试 | **2 个纯数据结构单测**：`board.rs:284-320`、`graph.rs:190-276`；**0 个**生命周期/取消/事件分流/工具接线测试；前端 0 测试 | `multi_agents_tests.rs`(4778 行)、`control_tests.rs`(6197)、`registry_tests.rs`(600)、`role_tests.rs`(826)、`spawn_publication_tests.rs`(359)、SQLite 测试、`core/tests/suite/` 约 25 个 collab/fork/mailbox 端到端 |

**D-31（多智能体测试盲区最严重）**
- **表现**：1499 行实现只有 2 个单测，且都测内存数据结构；`Task`/`spawn_agent`/`subagent::run`/board 工具/api 层 0 测试。
- **原因**：没有 mock LLM + 内存 SQLite 的集成夹具（§10）。
- **影响**：D-21（重复工具）、D-23（role 丢弃）、D-28（休眠能力）全部无法被自动发现——与 §5 的 D-01 同一模式：**函数写对了，接线错了**。

### 16.9 专项差异矩阵（与 §11 合并阅读）

| ID | 差异结论 | 严重度 | 证据 |
|---|---|---|---|
| **D-21** | `Task` 与 `spawn_agent` 是同一 `subagent::run` 的两个入口，`read_only` 相反（true/false），模型选择直接影响能否并行 | **P1** | `task.rs:8-11,101`、`spawn_agent.rs:9-16,67` |
| **D-22** | 无 `AgentStatus` 状态机、无 `AgentControl` 接口；无 wait/send/resume/close/interrupt/list，子代理是阻塞调用栈 | **P1** | `subagent.rs:133-247`；`C:core/src/agent/api.rs:41-161` |
| **D-23** | 无子代理角色系统，`role`/`nickname` 被丢弃；`AgentBundle` 是会话级人格配置且与子代理不相交；三套硬编码身份 | **P1** | `spawn_agent.rs:57-65`、`subagent.rs:148-149`、`board.rs:123-127`、`collaboration.rs:48-49` |
| **D-24** | 不 fork 父历史、子事件用完即删，无 `ForkPersistence` / cold resume | P2 | `subagent.rs:180-187`；`C:core/src/agent/history.rs:161-483` |
| **D-25** | board 是 UI 展示级；完成报告只 emit 事件不注入父上下文；无 mailbox / `InterAgentCommunication` / `trigger_turn` 唤醒 | **P1** | `subagent.rs:218-233`、`events.rs:172-183`；`C:control/completion.rs:27-155` |
| **D-26** | graph/board 进程内 `OnceLock`+`HashMap`，重启丢失；事件一次性推送无重放 | P1 | `graph.rs:180-184`、`board.rs:274-278` |
| **D-27** | 子代理不调 `begin_turn`，`request_cancel(sub_id)` 无效，只能整父会话取消；无 spawn 中途清理 | P1 | `subagent.rs:164-184`；全仓 `begin_turn` 仅 3 处 |
| **D-28** | `create_channel`/`search_posts`/`BoardChannelCreated` 休眠；无 create_thread；`with_board_read` 用写锁；BFS 过滤剪枝后代 | P2 | `board.rs:108-117,120-148,246-271`、`graph.rs:146-162` |
| **D-29** | 5 个专用事件通道 + 字符串 phase，非 Item 生命周期；前端模块级 Map 无自愈 | P2 | `events.rs:82-194`、`subagents.ts` |
| **D-30** | `branch` 不是 fork：不落库、无工具、前端全量上传历史 | P2 | `branch.rs:1-9,80-129`；`C:thread_manager.rs:1801,1819-1822` |
| **D-31** | 多智能体 1499 行仅 2 个数据结构单测，0 生命周期/集成测试 | **P1** | `board.rs:284`、`graph.rs:190` |

### 16.10 专项结论

1. **Nova 的"多智能体"本质是"并行只读研究子任务"，不是"多个可寻址、可协作的 Agent"**。核心证据：`subagent::run` 只是把 `AgentEngine::execute_turn` 换一个派生 ID 跑一次，返回报告后删除全部痕迹；没有实体、状态、消息地址、生命周期接口。codex 29418 行里相当一部分正是在建这套基础设施（registry / mailbox / graph store / role / AgentControl / fork）。
2. **`Task` 是实际可用且设计良好的那个入口**：`read_only=true` 让它能进只读批并行、hooks 完整、description 明确写了"不要用它做写入/单次小查询"、报告 16000 字符截断。问题在于它与 `spawn_agent` 重复，而后者承诺的 role/nickname 不兑现——**建议二选一，保留 `Task` 并删掉 `spawn_agent`，或把 `spawn_agent` 改成真正的异步派发**。
3. **board 不是 agent 通信层，是 UI 协同面板**。它无法唤醒父代理、不注入模型上下文、不持久化。若要做真正的多智能体协作，缺的不是 board，而是 mailbox + `InterAgentCommunication` + `trigger_turn` 这三件套。
4. **最省力的改进顺序**：删重复工具 → 修 role 丢弃/身份三套 → 补 3 条集成测试 → 把 graph/board 落 SQLite → 再考虑异步生命周期。前两步是"删代码/改参数"，成本极低、面试收益最高。

### 16.11 对 §3.8 / §4.2.5 的修正

| 位置 | 原说法 | 修正 |
|---|---|---|
| §3.8 | "codex：multi-agents v1 + v2 + AgentControl + AgentRegistry + agent-graph-store + agent-roles + fork" | 成立但需限定：`agent-identity`、`collaboration-mode-templates`、`runtime-agent-instance`、`ext/agent` **与子代理无关**；`multi_agent_tool.rs` 不是 legacy 而是 V2 命名空间包装器；不存在 `AgentRole`/`AgentType` 枚举 |
| §3.8 | "Nova：`subagent.rs` 复用 `AgentEngine::execute_turn`…递归防护靠白名单排除 Task/spawn_agent，无显式深度计数" | 后半成立；补充：白名单同时排除了 `mcp_auth`，子代理**没有 MCP 能力**；嵌套 `:sub:` 仍会被识别为子代理 |
| §4.2.5 | "`Task`…`spawn_agent`…`board_*`" 分三条并列 | 需强调 `Task` 与 `spawn_agent` 调**同一个** `subagent::run`，差异只有 hooks 与 `read_only`；`spawn_agent` 的 role/nickname 被丢弃 |
| §4.2.5 | "codex 对应 `agent-message-board-host` 与 v2 `send_message`" | message board 是四层子系统（`message-board` trait/后端 ← `ext/agent-message-board` ← host ← client，`message-board-mcp` 只是 MCP 适配），与 mailbox 是两条独立通道 |
| §11 D-19 | "多智能体单版本 + 进程内 board/graph，无持久化/异步协作/深度计数" | 结论方向对，但粒度不足；拆成 D-21～D-31 共 11 条，其中 6 条 P1 |
| §12 P2-4 | "多智能体 v2…目前单版本够面试讲述" | **上调为 P1**：不是因为要追 v2，而是因为 D-21/D-23（重复工具、role 空壳）会在面试追问时直接暴露，且修复成本极低 |


