// wrapper 剥离工具。
//
// strip_safe_wrappers 用于把命令字符串中的安全 wrapper（timeout/time/nice/
// nohup/stdbuf）和 SAFE_ENV_VARS 前缀剥掉，返回被包装的真实命令文本。
// 主要用于权限签名生成和 UI 预览，让用户看到实际要执行的命令。
//
// strip_wrappers_from_argv 在 argv 层面做同样的事，用于语义检查。

pub use crate::llm::utils::bash_ast::semantics::strip_wrappers_from_argv;

/// 会执行任意字符串作为代码的 shell builtin 集合。
/// 用于快速查找，避免在多处重复定义。
pub const EVAL_LIKE_BUILTINS: &[&str] = &[
    "eval",
    "source",
    ".",
    "exec",
    "command",
    "builtin",
    "fc",
    "coproc",
    "noglob",
    "nocorrect",
    "trap",
    "enable",
    "mapfile",
    "readarray",
    "hash",
    "bind",
    "complete",
    "compgen",
    "alias",
    "let",
];

/// read-only 命令 allowlist：这些命令在解析成功且语义安全时自动放行。
/// 命令名匹配 argv[0]（wrapper 剥离后）。
pub const READ_ONLY_COMMANDS: &[&str] = &[
    // 文件查看
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "file",
    "stat",
    "du",
    "df",
    "ls",
    "dir",
    "tree",
    "find",
    "locate",
    // 搜索
    "grep",
    "rg",
    "ag",
    "ack",
    "fgrep",
    "egrep",
    // git 只读
    "git", // git 需要子命令检查，单独处理
    // diff
    "diff",
    // echo/printf（只输出，但 printf 有 -v 风险，已在 semantics 拦截）
    "echo",
    "printf",
    // 环境查看
    "env",
    "printenv",
    "which",
    "whereis",
    "type",
    "command",
    // 进程查看
    "ps",
    "top",
    "htop",
    "jobs",
    "fg",
    "bg",
    // 网络查看
    "ping",
    "host",
    "dig",
    "nslookup",
    "traceroute",
    "tracepath",
    // 系统查看
    "uname",
    "uptime",
    "whoami",
    "id",
    "date",
    "cal",
    // 文本处理（只读）
    "sort",
    "uniq",
    "cut",
    "tr",
    "sed", // sed -i 会写文件，但 semantics 不拦截，需要路径检查
    "awk", // awk 可以 system()，但 semantics 已拦截
    // JSON 处理（只读）
    "jq",
    "python", // python -c 会执行代码，semantics 应该拦截 -c
    "python3",
    "node",
    // 版本查看
    "version",
    "--version",
    "-V",
    // PowerShell 只读
    "Get-ChildItem",
    "Get-Content",
    "Get-Location",
];

/// git 的只读子命令。
pub const GIT_READ_ONLY_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "branch",
    "branches",
    "tag",
    "tags",
    "remote",
    "remotes",
    "stash",
    "list",
    "blame",
    "shortlog",
    "describe",
    "rev-parse",
    "rev-list",
    "ls-files",
    "ls-tree",
    "ls-remote",
    "config",
    "--get",
    "reflog",
    "fsck",
    "count-objects",
    "grep",
];

/// 检查命令是否是 read-only（无需审批即可放行）。
/// 输入是 wrapper 剥离后的 argv。
pub fn is_read_only_command(argv: &[String]) -> bool {
    if argv.is_empty() {
        return false;
    }

    let name = argv[0].as_str();

    // git 需要子命令检查
    if name == "git" {
        if argv.len() < 2 {
            return true; // bare git 显示帮助
        }
        let subcommand = argv[1].as_str();
        // git --version 等只读
        if subcommand.starts_with('-') {
            return true;
        }
        return GIT_READ_ONLY_SUBCOMMANDS.contains(&subcommand);
    }

    // echo/printf：semantics 已保证没有 -v 之类的危险 flag
    // 但 echo 可能被用于构造脚本，这里仍然放行（执行不写文件）
    READ_ONLY_COMMANDS.contains(&name)
}
