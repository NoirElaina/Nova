// bash 命令的 AST 解析与语义检查。
//
// 核心设计：用 tree-sitter-bash 把命令解析成 AST，对节点类型做 fail-closed
// allowlist 校验。任何无法静态分析的节点（命令替换、子 shell、控制流、算术
// 展开等）都标为 TooComplex，调用方应当询问用户而非放行。
//
// 这不是沙箱。它只回答一个问题：能否为命令字符串中的每个简单命令生成
// 可信的 argv？如果能，下游可以做 read-only allowlist 匹配和路径约束检查。
// 如果不能，就必须 ask 用户。
//
// 权限判定的编排逻辑在 permissions/mod.rs 的 check_command 里，这里只提供
// 解析、语义检查、wrapper 剥离三个原语。

pub mod parser;
pub mod semantics;
pub mod types;
pub mod wrappers;
