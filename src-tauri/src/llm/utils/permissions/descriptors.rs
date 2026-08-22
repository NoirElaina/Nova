//! 工具权限描述符构建：内置工具通过这两个 helper 声明操作的风险级别。
//!
//! 描述符只描述"这个操作是什么、风险多高"，是否审批由审批策略决定。

use serde_json::Value;

use crate::llm::tools::ToolPermissionDescriptor;
use crate::llm::utils::permissions::checks::{
    check_command, check_file_path, normalize_command_for_match, normalize_path_for_match,
    truncate_chars,
};
use crate::llm::utils::permissions::RiskLevel;

/// shell 命令类工具（Bash/PowerShell 等）的权限描述。
pub(crate) fn describe_shell_command_permission(
    tool_name: &str,
    preview_label: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    // command: 当前工具请求执行的终端命令文本。
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();

    if command.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:<empty>", tool_name),
            preview: "命令为空".to_string(),
            warning: Some("命令为空，无法执行。".to_string()),
            risk: RiskLevel::Forbidden,
        });
    }

    let normalized = normalize_command_for_match(command);
    // 用 AST 引擎判定命令风险：Safe 放行，Risky 交由审批策略，Forbidden 硬拒绝。
    let (risk, warning) = check_command(command);

    Some(ToolPermissionDescriptor {
        signature: format!("{}:{}", tool_name, normalized),
        preview: format!(
            "{}（{}）：{}",
            preview_label,
            tool_name,
            truncate_chars(command, 180)
        ),
        warning,
        risk,
    })
}

/// 文件写类工具（Write/Edit/MultiEdit 等）的权限描述。
/// 受保护/敏感路径一律 Forbidden（硬拒绝），其余 Safe。
pub(crate) fn describe_file_write_permission(
    tool_name: &str,
    preview_label: &str,
    path_key: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    // path: 当前写操作的目标路径；不同工具可通过 path_key 复用这个 helper。
    let path = input
        .get(path_key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();

    if path.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:<empty>", tool_name),
            preview: "路径为空".to_string(),
            warning: Some("目标路径为空，无法执行。".to_string()),
            risk: RiskLevel::Forbidden,
        });
    }

    let normalized = normalize_path_for_match(path);
    // warning: 路径命中受保护目录或敏感标记时生成风险提示。
    let (risk, warning) = match check_file_path(path) {
        Ok(()) => (RiskLevel::Safe, None),
        Err(reason) => (RiskLevel::Forbidden, Some(reason)),
    };

    Some(ToolPermissionDescriptor {
        signature: format!("{}:{}", tool_name, normalized),
        preview: format!(
            "{}（{}）：{}",
            preview_label,
            tool_name,
            truncate_chars(path, 200)
        ),
        warning,
        risk,
    })
}
