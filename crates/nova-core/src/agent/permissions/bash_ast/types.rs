// bash AST 解析的类型定义。

use serde::Serialize;

/// 重定向操作符
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RedirectOp {
    Out,        // >
    Append,     // >>
    In,         // <
    OutAnd,     // >&
    InAnd,      // <&
    OutClobber, // >|
    AndOut,     // &>
    AndAppend,  // &>>
    Here,       // <<<
}

/// 重定向
#[derive(Debug, Clone, Serialize)]
pub struct Redirect {
    pub op: RedirectOp,
    pub target: String,
    pub fd: Option<u32>,
}

/// 简单命令：argv[0] 是命令名，其余是已解析引号的参数
#[derive(Debug, Clone, Serialize)]
pub struct SimpleCommand {
    /// 已解析引号的参数数组
    pub argv: Vec<String>,
    /// 前导 VAR=val 赋值
    pub env_vars: Vec<(String, String)>,
    /// 输入/输出重定向
    pub redirects: Vec<Redirect>,
    /// 原始源码 span（用于 UI 显示）
    pub text: String,
}

/// AST 解析结果
#[derive(Debug, Clone)]
pub enum ParseForSecurityResult {
    /// 成功解析，commands 是扁平化的简单命令列表
    Simple { commands: Vec<SimpleCommand> },
    /// 解析成功但含无法静态分析的节点（命令替换、子 shell、控制流等）
    TooComplex { reason: String },
    /// tree-sitter 不可用
    ParseUnavailable,
}

