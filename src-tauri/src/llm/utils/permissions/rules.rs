//! 持久化权限规则：跨会话记住用户的允许/拒绝决定（permission_rules.json）。
//!
//! 规则按操作签名匹配；对 shell 命令类签名额外支持前缀语义：
//! 规则 `Bash:git status` 同时匹配 `git status --short` 等追加参数的命令，
//! 对齐 "允许这一类命令" 的直觉（类似 codex execpolicy 的命令模式）。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::checks::normalize_command_for_match;

const PERMISSION_RULES_FILE_NAME: &str = "permission_rules.json";

/// shell 命令类工具名前缀：其签名的冒号后半段按"命令"做前缀匹配。
const SHELL_TOOL_PREFIXES: &[&str] = &["bash", "powershell", "pwsh", "shell", "terminal"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Allow,
    Deny,
}

/// 一条持久化权限规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    pub kind: RuleKind,
    /// 操作签名（与审批描述符 signature 同一归一化规则）。
    pub signature: String,
    pub created_at_ms: u64,
}

fn rules_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(PERMISSION_RULES_FILE_NAME))
        .map_err(|e| format!("Failed to resolve app_data_dir for permission rules: {}", e))
}

pub fn load_rules(app: &AppHandle) -> Vec<PermissionRule> {
    let Ok(path) = rules_path(app) else {
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

fn save_rules(app: &AppHandle, rules: &[PermissionRule]) -> Result<(), String> {
    let path = rules_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    crate::llm::utils::atomic_write::write_str(&path, &content)
        .map_err(|e| format!("failed to persist permission rules: {}", e))
}

/// 追加一条规则（同签名同类型的旧规则先移除，避免重复）。
pub fn add_rule(app: &AppHandle, kind: RuleKind, signature: &str) -> Result<(), String> {
    let mut rules = load_rules(app);
    rules.retain(|rule| !(rule.signature == signature && rule.kind == kind));
    rules.push(PermissionRule {
        kind,
        signature: signature.to_string(),
        created_at_ms: now_millis(),
    });
    save_rules(app, &rules)
}

/// 按签名删除规则（规则管理界面用）。
pub fn remove_rule(app: &AppHandle, signature: &str) -> Result<bool, String> {
    let mut rules = load_rules(app);
    let before = rules.len();
    rules.retain(|rule| rule.signature != signature);
    if rules.len() == before {
        return Ok(false);
    }
    save_rules(app, &rules)?;
    Ok(true)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把签名拆成 (工具前缀, 载荷)，如 `Bash:git status` → ("bash", "git status")。
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

    let (rule_tool, rule_cmd) = split_signature(rule_signature).unwrap();
    let (op_tool, op_cmd) = split_signature(operation_signature).unwrap();
    if !rule_tool.eq_ignore_ascii_case(op_tool) {
        return false;
    }

    let rule_cmd = normalize_command_for_match(rule_cmd);
    let op_cmd = normalize_command_for_match(op_cmd);
    op_cmd == rule_cmd
        || op_cmd
            .strip_prefix(&rule_cmd)
            .is_some_and(|rest| rest.starts_with(' '))
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
        assert!(rule_matches("Bash:git status", "Bash:git status"));
        assert!(!rule_matches("Bash:git status", "Bash:git log"));
    }

    #[test]
    fn shell_prefix_match() {
        assert!(rule_matches("Bash:npm run build", "Bash:npm run build --watch"));
        assert!(rule_matches("Bash:npm run build", "bash:npm run build"));
        // 前缀必须落在参数边界，避免 git statusx 被 git status 规则命中。
        assert!(!rule_matches("Bash:git stat", "Bash:git status"));
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
                signature: "Bash:npm run".into(),
                created_at_ms: 1,
            },
            PermissionRule {
                kind: RuleKind::Deny,
                signature: "Bash:npm run".into(),
                created_at_ms: 2,
            },
        ];
        let matched = find_matching_rule(&rules, "Bash:npm run dev").unwrap();
        assert_eq!(matched.kind, RuleKind::Deny);
    }

    #[test]
    fn allow_matched_by_prefix_rule() {
        let rules = vec![PermissionRule {
            kind: RuleKind::Allow,
            signature: "Bash:cargo test".into(),
            created_at_ms: 1,
        }];
        assert!(find_matching_rule(&rules, "Bash:cargo test --release").is_some());
        assert!(find_matching_rule(&rules, "Bash:cargo build").is_none());
    }
}
