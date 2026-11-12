//! 工具权限描述符构建：内置工具通过这两个 helper 声明操作的风险级别。
//!
//! 描述符只描述"这个操作是什么、风险多高"，是否审批由审批策略决定。

use serde_json::Value;

use crate::agent::tools::ToolPermissionDescriptor;
use crate::agent::permissions::checks::{
    check_command, check_file_path, normalize_command_for_match, normalize_path_for_match,
    truncate_chars,
};
use crate::agent::permissions::RiskLevel;

/// shell 命令类工具（exec_command / PowerShell 等）的权限描述。
pub(crate) fn describe_shell_command_permission(
    tool_name: &str,
    preview_label: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    // command: 当前工具请求执行的终端命令文本。
    let command = input
        .get("cmd").or_else(|| input.get("command"))
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

/// 交互式终端写入（`write_stdin`）的权限描述。
///
/// 这是 Codex `stdin_approval` 的等价物：一旦某个会话被批准，后续写入可以
/// 把任意命令送进仍在运行的 shell/REPL，因此非空写入必须归类为 Risky，
/// 只有空字符串（纯轮询）与 Ctrl+C（`\u0003`）才允许不打扰用户。
pub(crate) fn describe_stdin_permission(
    tool_name: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    let session_id = input
        .get("session_id")
        .map(|v| {
            v.as_i64()
                .map(|n| n.to_string())
                .or_else(|| v.as_u64().map(|n| n.to_string()))
                .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| (f as i64).to_string()))
                .or_else(|| {
                    v.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "?".to_string())
        })
        .unwrap_or_else(|| "?".to_string());

    let chars = input.get("chars").and_then(|v| v.as_str()).unwrap_or_default();
    if chars.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:{}:poll", tool_name, session_id),
            preview: format!("读取终端会话 {} 的新增输出", session_id),
            warning: None,
            risk: RiskLevel::Safe,
        });
    }
    if chars == "\u{3}" {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:{}:interrupt", tool_name, session_id),
            preview: format!("向终端会话 {} 发送 Ctrl+C", session_id),
            warning: None,
            risk: RiskLevel::Safe,
        });
    }

    // 与 exec_command 对齐：权限签名绑定到具体标准化输入，避免放行一条 write_stdin 后
    // 同一 session_id 的所有后续命令都自动免审。
    let normalized = normalize_command_for_match(chars);
    Some(ToolPermissionDescriptor {
        signature: format!("{}:{}:input:{}", tool_name, session_id, normalized),
        preview: format!(
            "向终端会话 {} 注入输入：{}",
            session_id,
            truncate_chars(&chars.replace(['\r', '\n'], "↵"), 120)
        ),
        warning: Some(
            "向仍在运行的终端/REPL 注入输入等同于在其中执行命令，可能修改文件或访问网络。"
                .to_string(),
        ),
        risk: RiskLevel::Risky,
    })
}

/// 文件写类工具（Write/Edit/MultiEdit 等）的权限描述。/// 受保护/敏感路径一律 Forbidden（硬拒绝），其余 Safe。
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_stdin_permission_scopes_signature_to_input() {
        let d_poll = describe_stdin_permission("write_stdin", &json!({ "session_id": " 1 ", "chars": "" })).unwrap();
        assert!(matches!(d_poll.risk, RiskLevel::Safe));
        assert_eq!(d_poll.signature, "write_stdin:1:poll");

        let d_ctrlc = describe_stdin_permission("write_stdin", &json!({ "session_id": 1, "chars": "\u{3}" })).unwrap();
        assert!(matches!(d_ctrlc.risk, RiskLevel::Safe));
        assert_eq!(d_ctrlc.signature, "write_stdin:1:interrupt");

        let d_ls = describe_stdin_permission("write_stdin", &json!({ "session_id": "1", "chars": "echo hi\n" })).unwrap();
        let d_rm = describe_stdin_permission("write_stdin", &json!({ "session_id": "1", "chars": "rm -rf /\n" })).unwrap();
        assert!(matches!(d_ls.risk, RiskLevel::Risky));
        assert!(matches!(d_rm.risk, RiskLevel::Risky));
        assert_ne!(d_ls.signature, d_rm.signature);
    }
}
