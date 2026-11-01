//! Nova 线协议：JSON-RPC 信封 + 全量 IPC 线类型。
//!
//! 这是桌面薄壳、CLI、守护进程共享的唯一类型事实源——
//! 桌面端只依赖本 crate，不链接任何内核实现。

pub mod agent;
pub mod llm;
pub mod mcp;
pub mod misc;
pub mod path_display;
pub mod rpc;
pub mod session;
pub mod settings;
