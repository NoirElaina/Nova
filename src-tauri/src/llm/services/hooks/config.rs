//! 声明式挂钩配置：hooks.toml 的强类型模型与加载。
//!
//! 配置文件位于 app_data_dir/hooks.toml。文件不存在 = 无任何钩子；
//! 解析失败通过后端错误事件上报并视为空配置，不静默吞掉错误。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

pub(crate) const HOOKS_FILE_NAME: &str = "hooks.toml";

/// hooks.toml 顶层结构。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HooksFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub hooks: HookEventsToml,
}

/// 12 个生命周期事件的挂钩分组表。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookEventsToml {
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<MatcherGroup>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<MatcherGroup>,
    #[serde(rename = "PostToolUseFailure", default)]
    pub post_tool_use_failure: Vec<MatcherGroup>,
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<MatcherGroup>,
    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<MatcherGroup>,
    #[serde(rename = "UserPromptSubmit", default)]
    pub user_prompt_submit: Vec<MatcherGroup>,
    #[serde(rename = "SubagentStart", default)]
    pub subagent_start: Vec<MatcherGroup>,
    #[serde(rename = "SubagentStop", default)]
    pub subagent_stop: Vec<MatcherGroup>,
    #[serde(rename = "PreCompact", default)]
    pub pre_compact: Vec<MatcherGroup>,
    #[serde(rename = "PostCompact", default)]
    pub post_compact: Vec<MatcherGroup>,
    #[serde(rename = "Stop", default)]
    pub stop: Vec<MatcherGroup>,
    #[serde(rename = "Error", default)]
    pub error: Vec<MatcherGroup>,
}

impl HookEventsToml {
    pub fn handler_count(&self) -> usize {
        self.all_groups()
            .into_iter()
            .flat_map(|(_, groups)| groups.iter())
            .map(|group| group.hooks.len())
            .sum()
    }

    /// 借用视图：按 (事件名, 分组列表) 遍历全部事件。
    pub fn all_groups(&self) -> [(&'static str, &Vec<MatcherGroup>); 12] {
        [
            ("PreToolUse", &self.pre_tool_use),
            ("PostToolUse", &self.post_tool_use),
            ("PostToolUseFailure", &self.post_tool_use_failure),
            ("SessionStart", &self.session_start),
            ("SessionEnd", &self.session_end),
            ("UserPromptSubmit", &self.user_prompt_submit),
            ("SubagentStart", &self.subagent_start),
            ("SubagentStop", &self.subagent_stop),
            ("PreCompact", &self.pre_compact),
            ("PostCompact", &self.post_compact),
            ("Stop", &self.stop),
            ("Error", &self.error),
        ]
    }
}

/// 一个匹配器分组：命中 matcher 的工具事件执行组内全部挂钩。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatcherGroup {
    /// 工具名匹配模式（大小写不敏感，支持 `*` 通配符）；
    /// 缺省或 `*` 表示匹配所有。非工具事件忽略该字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    #[serde(default)]
    pub hooks: Vec<HookHandlerConfig>,
}

/// 挂钩处理器：声明式的单个动作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HookHandlerConfig {
    /// 执行外部命令：事件 JSON 经 stdin 传入；
    /// 退出码 0 = 通过（stdout 作为附加上下文），2 = 拦截（stderr 作为原因），
    /// 其他非零 = 记录警告并忽略。
    #[serde(rename = "command", rename_all = "camelCase")]
    Command {
        command: String,
        /// Windows 平台的替代命令；当前平台为 Windows 且提供时优先使用。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_windows: Option<String>,
        /// 超时秒数；缺省 30 秒，超时杀进程并按拦截处理。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_sec: Option<u64>,
        /// 异步执行：发射后不等待、不影响任何结果。
        #[serde(default, rename = "async")]
        fire_and_forget: bool,
    },
    /// 注入一条上下文消息。文本支持占位符：
    /// {tool_name} {conversation_id} {subagent_name} {stop_reason} {error}。
    #[serde(rename = "context")]
    Context { text: String },
    /// 拦截当前工具调用（仅 PreToolUse 语义）。
    #[serde(rename = "block")]
    Block { reason: String },
    /// 输出/助手文本包含 pattern 时终止续跑。
    #[serde(rename = "stopWhen")]
    StopWhen { pattern: String },
    /// 工具失败时终止续跑（仅 PostToolUseFailure 语义）。
    #[serde(rename = "stopOnError")]
    StopOnError,
    /// 助手消息数超过 limit 时终止续跑（仅 Stop 语义）。
    #[serde(rename = "maxAssistantMessages")]
    MaxAssistantMessages { limit: usize },
    /// 把文本附加到 stop_reason / 错误信息末尾（SessionEnd / Error 语义）。
    #[serde(rename = "appendStopReason")]
    AppendStopReason { text: String },
}

pub(crate) fn hooks_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(HOOKS_FILE_NAME))
        .map_err(|e| format!("Failed to resolve app_data_dir for hooks.toml: {}", e))
}

/// 加载并解析 hooks.toml。
/// - 文件不存在：返回空配置（无任何挂钩）。
/// - 读取/解析失败：上报后端错误事件并返回空配置，避免坏配置阻塞全部工具调用。
pub(crate) fn load_hooks_file(app: &AppHandle) -> HooksFile {
    let path = match hooks_file_path(app) {
        Ok(path) => path,
        Err(error) => {
            crate::llm::utils::error_event::emit_backend_error(
                app,
                "hooks.config",
                error,
                Some("load"),
            );
            return HooksFile::default();
        }
    };

    if !path.exists() {
        return HooksFile::default();
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            crate::llm::utils::error_event::emit_backend_error(
                app,
                "hooks.config",
                format!("Failed to read {}: {}", path.display(), error),
                Some("load"),
            );
            return HooksFile::default();
        }
    };

    if raw.trim().is_empty() {
        return HooksFile::default();
    }

    match toml::from_str::<HooksFile>(&raw) {
        Ok(file) => file,
        Err(error) => {
            crate::llm::utils::error_event::emit_backend_error(
                app,
                "hooks.config",
                format!("hooks.toml 解析失败，挂钩已停用：{}", error),
                Some("parse"),
            );
            tracing::warn!(error = %error, path = %path.display(), "hooks.toml parse failed");
            HooksFile::default()
        }
    }
}

/// 校验 TOML 文本（前端保存前调用）：解析成功且返回处理器总数。
pub(crate) fn validate_hooks_toml(raw: &str) -> Result<usize, String> {
    if raw.trim().is_empty() {
        return Ok(0);
    }
    let file: HooksFile =
        toml::from_str(raw).map_err(|error| format!("hooks.toml 解析失败：{}", error))?;
    Ok(file.hooks.handler_count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_example() {
        let raw = r#"
description = "example"

[[hooks.PreToolUse]]
matcher = "bash"

  [[hooks.PreToolUse.hooks]]
  type = "command"
  command = "pwsh -File C:/checks.ps1"
  timeoutSec = 30

  [[hooks.PreToolUse.hooks]]
  type = "context"
  text = "执行工具前请再次确认参数"

  [[hooks.PreToolUse.hooks]]
  type = "block"
  reason = "该工具已被策略禁用"

[[hooks.PostToolUse]]
  [[hooks.PostToolUse.hooks]]
  type = "stopWhen"
  pattern = "FATAL"

[[hooks.Stop]]
  [[hooks.Stop.hooks]]
  type = "maxAssistantMessages"
  limit = 12
"#;
        let file: HooksFile = toml::from_str(raw).expect("parse");
        assert_eq!(file.description.as_deref(), Some("example"));
        assert_eq!(file.hooks.pre_tool_use.len(), 1);
        assert_eq!(file.hooks.pre_tool_use[0].matcher.as_deref(), Some("bash"));
        assert_eq!(file.hooks.pre_tool_use[0].hooks.len(), 3);
        assert_eq!(file.hooks.handler_count(), 5);
    }

    #[test]
    fn empty_file_is_empty() {
        let file = HooksFile::default();
        assert_eq!(file.hooks.handler_count(), 0);
    }

    #[test]
    fn rejects_unknown_fields() {
        let raw = r#"
[[hooks.PreToolUse]]
unknown = 1
"#;
        assert!(toml::from_str::<HooksFile>(raw).is_err());
    }

    #[test]
    fn rejects_unknown_handler_type() {
        let raw = r#"
[[hooks.PreToolUse]]
  [[hooks.PreToolUse.hooks]]
  type = "nope"
"#;
        assert!(toml::from_str::<HooksFile>(raw).is_err());
    }

    #[test]
    fn validates_command_windows_and_async() {
        let raw = r#"
[[hooks.PreToolUse]]
  [[hooks.PreToolUse.hooks]]
  type = "command"
  command = "sh check.sh"
  commandWindows = "pwsh check.ps1"
  async = true
"#;
        let file: HooksFile = toml::from_str(raw).expect("parse");
        match &file.hooks.pre_tool_use[0].hooks[0] {
            HookHandlerConfig::Command {
                command_windows,
                fire_and_forget,
                ..
            } => {
                assert_eq!(command_windows.as_deref(), Some("pwsh check.ps1"));
                assert!(*fire_and_forget);
            }
            other => panic!("unexpected handler: {:?}", other),
        }
    }
}
