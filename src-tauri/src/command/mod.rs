// 设置相关 tauri 命令入口。
pub mod settings;
// 设置中的敏感字段加密/解密。
pub mod settings_secrets;
// 会话历史与 compact 相关命令入口。
pub mod history;
// 会话文件变更审查与回退命令入口。
pub mod file_changes;
// 原生 LSP 语言服务状态与诊断命令入口。
pub mod lsp;
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
// 内置浏览器 WebView 控制命令入口。
pub mod browser;
// 主工作区文件树只读命令入口。
pub mod workspace;
