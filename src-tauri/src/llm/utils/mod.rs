// llm/utils 模块入口：负责 Nova 的 LLM 运行时辅助功能。
// 这里按职责拆分为不同子模块，并在上层通过 `use crate::llm::utils::*` 进行调用。

// 加载系统提示 (system prompt)，包括 plan mode 附加内容。
// 解析 system_prompt 文件并按模式拼装提示词。
pub mod system_prompt;

// 工具权限管理、用户鉴权、审批状态存储，和 tool 执行前检查紧密关联。
// 提供工具执行前的风控判定与审批状态消费。
pub mod permissions;

// 对话会话恢复逻辑：构建被插入到 current_messages 中的恢复上下文。
// 负责从历史边界提取摘要并生成恢复消息。
pub mod session_restore;

// 模型上下文窗口与输出 token 查询：从 litellm JSON 数据库按模型名精确/前缀匹配。
// 提供 get_context_window_tokens / get_max_output_tokens，未命中时返回保守默认值。
pub mod model_context;

// 模型价格与每轮 token 成本计算。使用 windowTokens/models.json 的内置价格数据。
pub mod pricing;

// 上下文组装入口：整合会话恢复与可选扩展上下文。
pub mod context_assembler;

// 统一后端错误事件输出到前端 telemetry 和 toast。
// 封装 backend-error 事件结构与发射方法。
pub mod error_event;

// 每轮 LLM 请求/响应原始数据，以 JSONL 格式追加写入 app_data_dir/turn_logs/。
pub mod turn_log;

// Tool-facing path helpers. File tools must use explicit absolute paths; workspace
// context is descriptive/default cwd only, not an implicit relative-path resolver.
pub mod paths;

// Write/Edit 工具共用的文件 I/O 辅助。
pub mod file_io;

// 原子写入：tempfile + fsync + rename + EXDEV fallback（记忆/会话文件 crash 安全）
pub mod atomic_write;
