// 设置相关 tauri 命令入口。
pub mod settings;
// 会话历史与 compact 相关命令入口。
pub mod history;
// MCP 服务管理相关命令入口。
pub mod mcp;
// 技能列表相关命令入口。
pub mod skill;
// RAG 知识库相关命令入口。
pub mod rag;
// 持久终端会话状态命令入口。
pub mod shell;
// AGENTS.md 配置读写命令入口。
pub mod agent_config;
// 定时任务（Cron）相关命令入口。
pub mod cron;
