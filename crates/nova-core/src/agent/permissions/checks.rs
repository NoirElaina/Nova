//! 静态风险检查：受保护路径、bash AST 命令判定、MCP 操作推断。
//!
//! 本模块只产出事实（Safe/Risky/Forbidden），不做策略裁决——
//! 是否审批由 ApprovalPolicy 与持久化规则在 mod.rs 决定。

use serde_json::Value;

use crate::agent::permissions::bash_ast::wrappers::{is_read_only_command, strip_wrappers_from_argv};
use crate::agent::permissions::RiskLevel;

// Path prefixes that should never be written without explicit override.
// 统一用正斜杠书写：normalize_path_for_match 会把待检查路径的 `\` 归一成 `/`，
// 两侧必须采用同一种分隔符，否则跨平台前缀匹配会静默失效。
const PROTECTED_PATH_PREFIXES: &[&str] = &[
    "c:/windows",
    "c:/program files",
    "c:/program files (x86)",
    "c:/programdata",
    "c:/users/public",
    "/etc",
    "/bin",
    "/sbin",
    "/usr",
    "/var",
    "/boot",
    "/system",
];

// Sensitive path markers that should be blocked even outside protected roots.
// 同样统一正斜杠形式，与 normalize_path_for_match 的归一化结果保持一致。
const PROTECTED_PATH_CONTAINS: &[&str] = &[
    "/.ssh/",
    "/.aws/",
    "/.gnupg/",
    "/.config/git",
    "/.git/config",
    // 整个 .git 目录都受保护，任何写入删除都拦截。
    "/.git/",
];

pub(crate) fn normalize_path_for_match(path: &str) -> String {
    // 用统一分隔符（正斜杠）与小写比较，减少跨平台路径写法差异。
    // 必须与 PROTECTED_PATH_PREFIXES / PROTECTED_PATH_CONTAINS 的书写形式一致，
    // 否则前缀/包含匹配会在某一平台静默失效。
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

pub(crate) fn normalize_command_for_match(command: &str) -> String {
    command
        // 压缩空白，避免同义命令因空格差异得到不同签名。
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        // 统一小写，减少大小写差异干扰。
        .to_ascii_lowercase()
}

pub(crate) fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect::<String>()
}

/// 检查路径是否触碰受保护路径。公开供 bash_ast 模块调用。
pub fn protected_path_violation(path: &str) -> Result<(), String> {
    check_file_path(path)
}

pub(crate) fn check_file_path(path: &str) -> Result<(), String> {
    let normalized = normalize_path_for_match(path);
    // normalized: 规范化后用于路径风险匹配的路径字符串。
    if normalized.is_empty() {
        return Err("Blocked by permission gate: target path is empty".to_string());
    }

    for prefix in PROTECTED_PATH_PREFIXES {
        // prefix: 当前检查的受保护路径前缀。
        // 前缀命中用于阻止系统目录写入。
        if normalized.starts_with(prefix) {
            return Err(format!(
                "Blocked by permission gate: writing protected path '{}'.",
                path
            ));
        }
    }

    for marker in PROTECTED_PATH_CONTAINS {
        // marker: 当前检查的敏感路径标记。
        // contains 命中用于阻止凭据/密钥等敏感目录。
        if normalized.contains(marker) {
            return Err(format!(
                "Blocked by permission gate: writing sensitive path '{}'.",
                path
            ));
        }
    }

    Ok(())
}

/// 对 bash 命令做 AST 级风险判定。
///
/// 返回值：
/// - Safe：命令通过所有检查（read-only allowlist 命中）
/// - Risky(warning)：命令需要关注，交由审批策略处理
/// - Forbidden(reason)：命令被硬性拒绝
pub(crate) fn check_command(command: &str) -> (RiskLevel, Option<String>) {
    use crate::agent::permissions::bash_ast::parser::parse_for_security;
    use crate::agent::permissions::bash_ast::semantics::check_semantics;

    let trimmed = command.trim();
    if trimmed.is_empty() {
        return (RiskLevel::Forbidden, Some("命令为空".to_string()));
    }

    #[cfg(target_os = "windows")]
    if let Some(res) = check_windows_powershell_fallback(command) {
        return res;
    }

    // 1. AST 解析 + fail-closed allowlist
    let parsed = match parse_for_security(command) {
        crate::agent::permissions::bash_ast::types::ParseForSecurityResult::Simple { commands } => commands,
        crate::agent::permissions::bash_ast::types::ParseForSecurityResult::TooComplex { reason } => {
            #[cfg(target_os = "windows")]
            if let Some(res) = check_windows_powershell_fallback(command) {
                return res;
            }
            return (
                RiskLevel::Risky,
                Some(format!("命令无法静态分析（{}），需要确认", reason)),
            );
        }
        crate::agent::permissions::bash_ast::types::ParseForSecurityResult::ParseUnavailable => {
            #[cfg(target_os = "windows")]
            if let Some(res) = check_windows_powershell_fallback(command) {
                return res;
            }
            return (
                RiskLevel::Risky,
                Some("bash 解析器不可用，无法静态分析命令安全性".to_string()),
            );
        }
    };

    if parsed.is_empty() {
        return (RiskLevel::Safe, None);
    }

    // 2. 语义检查（wrapper 剥离 + eval-like builtin 拦截）
    for cmd in &parsed {
        if let Err(reason) = check_semantics(cmd) {
            return (
                RiskLevel::Risky,
                Some(format!("命令含潜在风险（{}），需要确认", reason)),
            );
        }
    }

    // 3. 路径约束检查（重定向目标 + 路径参数）
    for cmd in &parsed {
        for redirect in &cmd.redirects {
            // 重定向目标含 shell 变量展开（$VAR 等）时，parser.rs 会把 target 标记为
            // __DYNAMIC__ 前缀。无法静态确定写入路径，必须 fail-closed 要求审批。
            // 攻击场景：echo x > $HOME/.ssh/authorized_keys
            //   bash 实际展开 $HOME 写入 ~/.ssh/authorized_keys，实现 SSH 后门。
            if redirect.target.starts_with("__DYNAMIC__") {
                return (
                    RiskLevel::Risky,
                    Some(format!(
                        "重定向目标含 shell 变量展开（{}），无法静态确定写入路径，需要确认",
                        &redirect.target["__DYNAMIC__".len()..]
                    )),
                );
            }
            if let Err(reason) = protected_path_violation(&redirect.target) {
                return (
                    RiskLevel::Forbidden,
                    Some(format!("重定向目标命中受保护路径：{}", reason)),
                );
            }
        }
        for arg in &cmd.argv {
            if arg.starts_with('-') {
                continue;
            }
            if let Err(reason) = protected_path_violation(arg) {
                return (
                    RiskLevel::Forbidden,
                    Some(format!("参数命中受保护路径：{}", reason)),
                );
            }
        }
    }

    // 4. read-only allowlist 检查
    // 对每个简单命令，剥 wrapper 后检查 argv[0] 是否在 allowlist 中。
    // 所有命令都必须是 read-only 才算 Safe；任一不是就升级为 Risky。
    for cmd in &parsed {
        if cmd.argv.is_empty() {
            continue;
        }
        let stripped = strip_wrappers_from_argv(&cmd.argv);
        if !is_read_only_command(&stripped) {
            #[cfg(target_os = "windows")]
            if let Some(res) = check_windows_powershell_fallback(command) {
                return res;
            }
            return (
                RiskLevel::Risky,
                Some(format!(
                    "命令 '{}' 不在只读 allowlist 中，需要确认",
                    cmd.argv.first().map(|s| s.as_str()).unwrap_or("?")
                )),
            );
        }
    }

    (RiskLevel::Safe, None)
}

#[cfg(target_os = "windows")]
fn check_windows_powershell_fallback(command: &str) -> Option<(RiskLevel, Option<String>)> {
    let lower = command.to_ascii_lowercase();

    // 1. 检查是否存在重定向符号写入文件
    if lower.contains('>') {
        return Some((
            RiskLevel::Risky,
            Some("命令包含重定向符号 (> / >>)，可能写入文件，需要确认".to_string()),
        ));
    }

    // 2. 检查危险命令与写操作关键字
    const POWERSHELL_DANGEROUS_TOKENS: &[&str] = &[
        "remove-item", "rmdir", "del", "erase", "clear-content",
        "set-content", "add-content", "out-file", "new-item",
        "copy-item", "move-item", "rename-item",
        "invoke-expression", "iex", "start-process", "saps",
        "invoke-webrequest", "iwr", "irm", "set-executionpolicy",
        "format-volume", "stop-process", "kill", "restart-computer", "stop-computer"
    ];

    // 分割管道
    let stages: Vec<&str> = command.split('|').map(|s| s.trim()).collect();
    if stages.is_empty() {
        return None;
    }

    const SAFE_POWERSHELL_CMDLETS: &[&str] = &[
        "get-childitem", "gci", "dir", "ls",
        "get-content", "gc", "cat", "type",
        "get-location", "gl", "pwd",
        "select-object", "select",
        "where-object", "where",
        "measure-object", "measure",
        "sort-object", "sort",
        "get-command", "gcm",
        "get-help", "help", "man",
        "get-date", "get-host",
        "get-process", "gps",
        "get-service", "gsv",
        "get-history", "history",
        "write-output", "echo", "write-host",
        "findstr", "head", "tail", "more", "less"
    ];

    for stage in &stages {
        let trimmed_stage = stage.trim();
        if trimmed_stage.is_empty() {
            return None;
        }
        let tokens: Vec<&str> = trimmed_stage.split_whitespace().collect();
        let first_token = tokens[0].to_ascii_lowercase();

        // 检查参数中是否有指向受保护路径的参数
        for token in &tokens[1..] {
            let clean_token = token.trim_matches(|c| c == '\'' || c == '"');
            if let Err(reason) = check_file_path(clean_token) {
                return Some((RiskLevel::Forbidden, Some(reason)));
            }
        }

        // 检查是否包含危险关键字
        for dang in POWERSHELL_DANGEROUS_TOKENS {
            if first_token == *dang || tokens.iter().any(|t| t.to_ascii_lowercase() == *dang) {
                return Some((
                    RiskLevel::Risky,
                    Some(format!("PowerShell 命令包含写/执行操作（{}），需要确认", dang)),
                ));
            }
        }

        // 检查首 token 是否在安全只读命令表中
        let is_safe = SAFE_POWERSHELL_CMDLETS.iter().any(|c| &first_token == c)
            || is_read_only_command(&[first_token.clone()]);
        if !is_safe {
            return None;
        }
    }

    Some((RiskLevel::Safe, None))
}

fn looks_like_shell_mcp(server: &str, tool: &str) -> bool {
    let s = format!(
        "{} {}",
        server.to_ascii_lowercase(),
        tool.to_ascii_lowercase()
    );
    // s: server+tool 的小写拼接字符串。
    ["bash", "shell", "powershell", "pwsh", "terminal"]
        .iter()
        // 关键字模糊匹配：适配不同 MCP server/tool 命名习惯。
        .any(|k| s.contains(k))
}

fn looks_like_file_mcp(server: &str, tool: &str) -> bool {
    let s = format!(
        "{} {}",
        server.to_ascii_lowercase(),
        tool.to_ascii_lowercase()
    );
    // s: server+tool 的小写拼接字符串。
    ["file", "filesystem", "fs", "write", "edit", "replace"]
        .iter()
        // 关键字命中即按文件写操作风控处理。
        .any(|k| s.contains(k))
}

fn pick_string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        // key: 当前尝试提取的字段名。
        if let Some(v) = value.get(*key).and_then(|v| v.as_str()) {
            // v: JSON 字段值。
            let trimmed = v.trim();
            // trimmed: 去掉前后空白后的字符串。
            if !trimmed.is_empty() {
                // 返回原始 JSON 字符串切片，零拷贝。
                return Some(trimmed);
            }
        }
    }
    None
}

/// MCP 操作风险推断：按 server/tool 命名识别 shell / 文件写操作后复用同一检查器。
/// 返回 (风险级别, 警告, 归一化签名后缀)。
pub(crate) fn assess_mcp_operation(
    tool_name: &str,
    server: &str,
    tool: &str,
    arguments: &Value,
) -> (RiskLevel, Option<String>, String) {
    if looks_like_shell_mcp(server, tool) {
        // 兼容不同 server 的参数命名。
        let command =
            pick_string_field(arguments, &["command", "cmd", "script"]).unwrap_or_default();
        // command: shell 操作中提取到的命令字符串。
        let (risk, warning) = check_command(command);
        let signature = format!(
            "{}:{}:{}",
            tool_name,
            server.to_ascii_lowercase(),
            normalize_command_for_match(command)
        );
        return (risk, warning, signature);
    }

    if looks_like_file_mcp(server, tool) {
        // 常见路径参数别名统一提取。
        let path = pick_string_field(
            arguments,
            &["path", "file", "file_path", "target", "target_path"],
        )
        .unwrap_or_default();
        // path: 文件操作中提取到的目标路径。
        let (risk, warning) = match check_file_path(path) {
            Ok(()) => (RiskLevel::Safe, None),
            Err(reason) => (RiskLevel::Forbidden, Some(reason)),
        };
        let signature = format!(
            "{}:{}:{}",
            tool_name,
            server.to_ascii_lowercase(),
            normalize_command_for_match(&arguments.to_string())
        );
        return (risk, warning, signature);
    }

    // 其他 MCP 操作：无专门风控，产出最低限度签名供策略与规则引用。
    (
        RiskLevel::Safe,
        None,
        format!(
            "{}:{}:{}",
            tool_name,
            server.to_ascii_lowercase(),
            normalize_command_for_match(&arguments.to_string())
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_normalization_unifies_separators() {
        assert_eq!(
            normalize_path_for_match(r"C:\Windows\System32"),
            "c:/windows/system32"
        );
    }

    #[test]
    fn protected_prefix_detected() {
        assert!(check_file_path("C:/Windows/notepad.exe").is_err());
        assert!(check_file_path("/etc/passwd").is_err());
        assert!(check_file_path("/home/user/project/src/main.rs").is_ok());
    }

    #[test]
    fn sensitive_marker_detected() {
        assert!(check_file_path("/home/user/.ssh/id_rsa").is_err());
        assert!(check_file_path(r"D:\repo\.git\config").is_err());
    }

    #[test]
    fn empty_command_is_forbidden() {
        let (risk, _) = check_command("   ");
        assert!(matches!(risk, RiskLevel::Forbidden));
    }

    #[test]
    fn command_signature_normalized() {
        assert_eq!(normalize_command_for_match("  Git   STATUS "), "git status");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_safe_commands_pass() {
        let (risk, _) = check_command("Get-ChildItem -Path d:/project");
        assert!(matches!(risk, RiskLevel::Safe));

        let (risk_pipeline, _) = check_command("Get-ChildItem | Select-Object -First 10");
        assert!(matches!(risk_pipeline, RiskLevel::Safe));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_risky_and_forbidden_commands_flagged() {
        let (risk, _) = check_command("Remove-Item file.txt");
        assert!(matches!(risk, RiskLevel::Risky));

        let (risk_redirect, _) = check_command("Get-ChildItem > out.txt");
        assert!(matches!(risk_redirect, RiskLevel::Risky));

        let (risk_protected, _) = check_command("Get-ChildItem C:/Windows");
        assert!(matches!(risk_protected, RiskLevel::Forbidden));
    }
}

