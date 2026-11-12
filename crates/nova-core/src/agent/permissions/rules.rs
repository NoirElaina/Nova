//! 持久化权限规则：跨会话记住用户的允许/拒绝决定（permission_rules.json）。
//!
//! 规则按操作签名匹配；对 shell 命令类签名额外支持前缀语义：
//! 规则 `exec_command:git status` 同时匹配 `git status --short` 等追加参数的命令，
//! 符合“允许这一类命令”的使用直觉。

use crate::core::Core;

use super::checks::{has_unquoted_shell_metachar, normalize_command_for_match};

const PERMISSION_RULES_FILE_NAME: &str = "permission_rules.json";

/// shell 命令类工具名前缀：其签名的冒号后半段按"命令"做前缀匹配。
/// `bash` 是旧工具名，保留在列表里以便升级后旧规则继续生效（见 `shell_tool_family`）。
const SHELL_TOOL_PREFIXES: &[&str] = &[
    "exec_command",
    "bash",
    "powershell",
    "pwsh",
    "shell",
    "terminal",
];

/// 把工具名归一到一个 shell 家族：`Bash` 与 `exec_command` 是同一个工具的前后两代，
/// 旧版 `permission_rules.json` 里写下的 `Bash:git status` 必须继续命中新工具。
fn shell_tool_family(tool: &str) -> &str {
    match tool {
        "bash" => "exec_command",
        other => other,
    }
}

pub use nova_protocol::misc::{PermissionRule, RuleKind};

fn rules_path(core: &Core) -> Result<std::path::PathBuf, String> {
    Ok(core.config().data_dir().join(PERMISSION_RULES_FILE_NAME))
}

pub fn load_rules(core: &Core) -> Vec<PermissionRule> {
    let Ok(path) = rules_path(core) else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<PermissionRule>>(&raw).unwrap_or_else(|error| {
        tracing::warn!(error = %error, path = %path.display(), "permission rules parse failed");
        Vec::new()
    })
}

fn save_rules(core: &Core, rules: &[PermissionRule]) -> Result<(), String> {
    let path = rules_path(core)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    crate::agent::utils::atomic_write::write_str(&path, &content)
        .map_err(|e| format!("failed to persist permission rules: {}", e))
}

/// 追加一条规则（同签名同类型的旧规则先移除，避免重复）。
pub fn add_rule(core: &Core, kind: RuleKind, signature: &str) -> Result<(), String> {
    let mut rules = load_rules(core);
    rules.retain(|rule| !(rule.signature == signature && rule.kind == kind));
    rules.push(PermissionRule {
        kind,
        signature: signature.to_string(),
        created_at_ms: now_millis(),
    });
    save_rules(core, &rules)
}

/// 按签名删除规则（规则管理界面用）。
pub fn remove_rule(core: &Core, signature: &str) -> Result<bool, String> {
    let mut rules = load_rules(core);
    let before = rules.len();
    rules.retain(|rule| rule.signature != signature);
    if rules.len() == before {
        return Ok(false);
    }
    save_rules(core, &rules)?;
    Ok(true)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把签名拆成 (工具前缀, 载荷)，如 `exec_command:git status` → ("exec_command", "git status")。
fn split_signature(signature: &str) -> Option<(&str, &str)> {
    signature.split_once(':')
}

fn is_shell_signature(signature: &str) -> bool {
    split_signature(signature)
        .map(|(tool, _)| {
            let tool = tool.to_ascii_lowercase();
            SHELL_TOOL_PREFIXES.iter().any(|prefix| tool == *prefix)
        })
        .unwrap_or(false)
}

/// 规则是否命中操作签名。
/// - 精确相等命中；
/// - shell 命令类签名额外支持前缀命中：规则命令是操作命令的前缀
///   且边界为空格（`git status` 命中 `git status --short`，不命中 `git statusx`）。
pub fn rule_matches(rule_signature: &str, operation_signature: &str) -> bool {
    if rule_signature == operation_signature {
        return true;
    }

    if !is_shell_signature(rule_signature) || !is_shell_signature(operation_signature) {
        return false;
    }

    let Some((rule_tool, rule_cmd)) = split_signature(rule_signature) else {
        return false;
    };
    let Some((op_tool, op_cmd)) = split_signature(operation_signature) else {
        return false;
    };
    // 旧 `Bash:` 规则与新 `exec_command:` 规则视为同一家族，升级后不失效。
    if !shell_tool_family(&rule_tool.to_ascii_lowercase())
        .eq(shell_tool_family(&op_tool.to_ascii_lowercase()))
    {
        return false;
    }

    let rule_cmd = normalize_command_for_match(rule_cmd);
    let op_cmd = normalize_command_for_match(op_cmd);
    if op_cmd == rule_cmd {
        return true;
    }
    let Some(rest) = op_cmd.strip_prefix(&rule_cmd) else {
        return false;
    };
    // 前缀必须是完整词边界；且剩余部分只能追加**参数**，
    // 不能借引号外的 `;` / `&&` / `|` / 重定向 / 命令替换引入第二条命令——
    // 同时允许 `--grep="a;b"` 等双/单引号内合法参数。
    rest.starts_with(' ') && !has_unquoted_shell_metachar(rest)
}

/// 在规则集中查找首个命中的规则（deny 优先于 allow）。
pub fn find_matching_rule<'a>(
    rules: &'a [PermissionRule],
    operation_signature: &str,
) -> Option<&'a PermissionRule> {
    if let Some(deny) = rules
        .iter()
        .find(|rule| rule.kind == RuleKind::Deny && rule_matches(&rule.signature, operation_signature))
    {
        return Some(deny);
    }
    rules
        .iter()
        .find(|rule| rule.kind == RuleKind::Allow && rule_matches(&rule.signature, operation_signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(rule_matches("exec_command:git status", "exec_command:git status"));
        assert!(!rule_matches("exec_command:git status", "exec_command:git log"));
    }

    #[test]
    fn shell_prefix_match() {
        assert!(rule_matches("exec_command:npm run build", "exec_command:npm run build --watch"));
        assert!(rule_matches("exec_command:npm run build", "EXEC_COMMAND:npm run build"));
        // 前缀必须落在参数边界，避免 git statusx 被 git status 规则命中。
        assert!(!rule_matches("exec_command:git stat", "exec_command:git status"));
        // 非 shell 工具不做前缀匹配。
        assert!(!rule_matches(
            "Write:/repo/a",
            "Write:/repo/a/b"
        ));
    }

    #[test]
    fn deny_wins_over_allow() {
        let rules = vec![
            PermissionRule {
                kind: RuleKind::Allow,
                signature: "exec_command:npm run".into(),
                created_at_ms: 1,
            },
            PermissionRule {
                kind: RuleKind::Deny,
                signature: "exec_command:npm run".into(),
                created_at_ms: 2,
            },
        ];
        let matched = find_matching_rule(&rules, "exec_command:npm run dev").unwrap();
        assert_eq!(matched.kind, RuleKind::Deny);
    }

    #[test]
    fn allow_matched_by_prefix_rule() {
        let rules = vec![PermissionRule {
            kind: RuleKind::Allow,
            signature: "exec_command:cargo test".into(),
            created_at_ms: 1,
        }];
        assert!(find_matching_rule(&rules, "exec_command:cargo test --release").is_some());
        assert!(find_matching_rule(&rules, "exec_command:cargo build").is_none());
    }

    /// 回归：旧版规则文件里写的是 `Bash:...`，工具改名后不能静默失效。
    #[test]
    fn legacy_bash_rules_still_match_exec_command() {
        assert!(rule_matches("Bash:git status", "exec_command:git status"));
        assert!(rule_matches("bash:npm run build", "exec_command:npm run build --watch"));
        assert!(rule_matches("exec_command:cargo test", "Bash:cargo test"));
        // 前缀边界仍然生效。
        assert!(!rule_matches("Bash:git stat", "exec_command:git status"));
    }

    /// 回归：换行曾被折叠成空格，导致 `git status\nrm -rf x` 命中 `git status` 的前缀规则。
    /// 测试必须覆盖真实 `\n`、`\r`、`&&` 与引号内分号。
    #[test]
    fn newline_does_not_extend_prefix_rule() {
        assert!(!rule_matches(
            "exec_command:git status",
            "exec_command:git status\nrm -rf build"
        ));
        assert!(!rule_matches(
            "exec_command:git status",
            "exec_command:git status\rrm -rf build"
        ));
        assert!(!rule_matches(
            "exec_command:git status",
            "exec_command:git status ; rm -rf build"
        ));
        assert!(!rule_matches(
            "exec_command:git status",
            "exec_command:git status && rm -rf build"
        ));
        assert!(!rule_matches(
            "exec_command:git log",
            "exec_command:git log --grep=\"$(rm -rf)\""
        ));
        assert!(rule_matches(
            "exec_command:git log",
            "exec_command:git log --grep=\"a;b\""
        ));
    }
}
