# New Tool Template

这个目录是 Nova 工具系统的模板包。

目标是让你新增工具时尽量只做两件事：

1. 复制一个合适的模板目录
2. 在 `src-tauri/src/llm/tools/mod.rs` 的 `declare_builtin_tools!` 里加一行

例如：

```rust
my_tool => "MyTool/mod.rs",
```

这行会同时完成：

- 模块声明
- 工具挂载

你不需要再额外改这些全局位置：

- 执行总路由
- 只读白名单
- 权限总表分支
- 后处理 special-case

## 目录说明

- `mod.rs`
  - 通用大模板，适合第一次做新工具时参考全部能力
- `ReadOnlyToolTemplate/mod.rs`
  - 只读、同步、无 `AppHandle` 的简单工具
- `AppToolTemplate/mod.rs`
  - 需要 `AppHandle`、异步执行或会话上下文的工具
- `PrivilegedToolTemplate/mod.rs`
  - 需要权限确认，或者有 side-channel 输出的复杂工具

## 什么时候用哪个模板

- 读文件、检索、解析类工具：优先用 `ReadOnlyToolTemplate`
- 要调用 Tauri 命令、MCP、数据库、运行时状态：优先用 `AppToolTemplate`
- 会改宿主环境、访问敏感资源、需要截图/额外消息回灌：优先用 `PrivilegedToolTemplate`

## 最小接入步骤

1. 复制一个模板目录，改成你的工具目录名
2. 把 `tool()` 里的：
   - `name`
   - `description`
   - `input_schema`
   改成真实定义
3. 实现 `execute()`，如果需要再实现：
   - `execute_with_app()`
   - `permission()`
   - `postprocess_output()`
4. 检查 `registration()` 选对了挂载方式
5. 在 `tools/mod.rs` 的 `declare_builtin_tools!` 中加一行
6. 运行：

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

## registration 选择规则

### `sync_tool(...)`

适合：

- 同步工具
- 不需要 `AppHandle`
- 不需要权限声明
- 不需要 side-channel 后处理

### `app_tool(...)`

适合：

- 需要 `AppHandle`
- 需要异步逻辑
- 需要 conversation scope
- 但不需要特殊权限和后处理

### `app_tool_with_extras(...)`

适合：

- 需要权限确认
- 需要 side-channel 消息
- 或者你想把复杂行为显式写在工具模块里

## 常见实现建议

### 1. 只读工具尽量标成 `read_only = true`

这样它们可以进入只读并发批处理，不会和写操作工具混在一起串行执行。

### 2. 返回值尽量稳定

推荐优先返回 JSON 字符串，例如：

```rust
json!({
    "ok": true,
    "result": data
}).to_string()
```

这样更方便前后续工具链处理，也更容易判断 `is_error`。

### 3. 权限描述要稳定

`signature` 最好能稳定描述“这次敏感操作是什么”，避免授权缓存失效过多或过少。

### 4. 后处理只做 side-channel

`postprocess_output()` 适合做：

- 截图转图片消息
- 额外上下文块
- 结构化结果补充

不要把主要业务逻辑塞进后处理。

## 一个最常见的新增工具流程

如果你要新增 `OpenProjectTool`：

1. 复制 `AppToolTemplate` 到 `OpenProjectTool`
2. 改 `tool().name = "open_project"`
3. 实现 `execute_with_app()`
4. 保留 `registration()`
5. 在 `declare_builtin_tools!` 中添加：

```rust
open_project_tool => "OpenProjectTool/mod.rs",
```

## 什么时候还需要改全局

通常不需要。

只有这几类情况可能还要动别处：

- 你要新增新的协议级消息类型
- 你要改工具执行调度策略
- 你要改 provider 暴露工具的整体方式
- 你要给工具系统新增新的元数据能力

如果只是普通新增工具，应该停留在“复制模板 + 挂一行”这个范围内。
