// 插件清单（plugin.json）的数据结构与校验。
// manifest 是插件对宿主的唯一静态声明：工具元数据、界面贡献、权限申请。
// 工具列表全部来自 manifest（而非运行时注册），保证不执行 JS 也能同步构建工具目录。

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_parameters() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

fn default_settings_view() -> String {
    "ui/settings.html".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default = "default_parameters")]
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsTab {
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "default_settings_view")]
    pub view: String,
}

/// 斜杠命令贡献：用户输入 /name 触发预置工作流（promptTemplate 展开为消息发送）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommandSpec {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// 消息模板，支持 {workspace}（当前会话工作区路径）与 {date}（当日日期）占位符。
    #[serde(default)]
    pub prompt_template: String,
}

/// 提示词片段贡献：向系统提示词注入领域知识（只增不改，AI 行为引导）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPromptSection {
    pub content: String,
    /// 拼接锚点：after-tools（主提示词后）/ before-memory（记忆快照后）/ end（末尾）。
    #[serde(default = "default_prompt_placement")]
    pub placement: String,
}

fn default_prompt_placement() -> String {
    "end".to_string()
}

/// 提示词片段长度上限：防止单插件用超大片段撑爆 token 预算。
pub const PROMPT_SECTION_MAX_CHARS: usize = 2000;
/// 斜杠命令模板长度上限。
pub const COMMAND_TEMPLATE_MAX_CHARS: usize = 8000;
/// 合法锚点白名单。
pub const PROMPT_PLACEMENTS: &[&str] = &["after-tools", "before-memory", "end"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributes {
    #[serde(default)]
    pub tools: Vec<PluginToolSpec>,
    #[serde(default)]
    pub settings_tab: Option<PluginSettingsTab>,
    #[serde(default)]
    pub commands: Vec<PluginCommandSpec>,
    #[serde(default)]
    pub prompt_section: Option<PluginPromptSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub contributes: PluginContributes,
    /// 系统内置插件标记：不可卸载，只可停用。
    #[serde(default)]
    pub system: bool,
    /// 更新检查源（指向新版本 zip 的 URL）。
    #[serde(default)]
    pub update_url: Option<String>,
}

fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && c == '_'))
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// 斜杠命令名：小写字母/数字/连字符，1-32 字符，字母开头。
fn valid_command_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && c == '-')
            })
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
}

/// 宿主内置斜杠命令名清单：插件命令不得与之重名（前端 slash-commands.ts 的 SLASH_COMMANDS）。
pub const BUILTIN_SLASH_COMMAND_NAMES: &[&str] = &["skill", "compact", "memory", "review", "init"];

impl PluginManifest {
    /// manifest 语义校验：插件 id / 工具名 / 命令名 / 提示词片段 / 权限声明格式。
    pub fn validate(&self) -> Result<(), String> {
        if !valid_plugin_id(&self.id) {
            return Err(format!(
                "invalid plugin id '{}': expected 1-64 chars of [A-Za-z0-9_-]",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            return Err(format!("plugin '{}' has empty name", self.id));
        }
        for tool in &self.contributes.tools {
            if !valid_tool_name(&tool.name) {
                return Err(format!(
                    "invalid tool name '{}' in plugin '{}': expected 1-64 chars of [a-z0-9_], starting with a letter",
                    tool.name, self.id
                ));
            }
            if tool.description.trim().is_empty() {
                return Err(format!(
                    "tool '{}' in plugin '{}' has empty description",
                    tool.name, self.id
                ));
            }
        }
        for command in &self.contributes.commands {
            if !valid_command_name(&command.name) {
                return Err(format!(
                    "invalid command name '{}' in plugin '{}': expected 1-32 chars of [a-z0-9-], starting with a letter",
                    command.name, self.id
                ));
            }
            if BUILTIN_SLASH_COMMAND_NAMES.contains(&command.name.as_str()) {
                return Err(format!(
                    "command '{}' in plugin '{}' conflicts with a builtin slash command",
                    command.name, self.id
                ));
            }
            if command.title.trim().is_empty() {
                return Err(format!(
                    "command '{}' in plugin '{}' has empty title",
                    command.name, self.id
                ));
            }
            if command.prompt_template.len() > COMMAND_TEMPLATE_MAX_CHARS {
                return Err(format!(
                    "command '{}' in plugin '{}' promptTemplate exceeds {} chars",
                    command.name, self.id, COMMAND_TEMPLATE_MAX_CHARS
                ));
            }
        }
        if let Some(section) = &self.contributes.prompt_section {
            if section.content.trim().is_empty() {
                return Err(format!(
                    "plugin '{}' declares an empty promptSection",
                    self.id
                ));
            }
            if section.content.len() > PROMPT_SECTION_MAX_CHARS {
                return Err(format!(
                    "plugin '{}' promptSection exceeds {} chars",
                    self.id, PROMPT_SECTION_MAX_CHARS
                ));
            }
            if !PROMPT_PLACEMENTS.contains(&section.placement.as_str()) {
                return Err(format!(
                    "plugin '{}' promptSection has invalid placement '{}' (expected one of {:?})",
                    self.id, section.placement, PROMPT_PLACEMENTS
                ));
            }
        }
        if let Some(url) = &self.update_url {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!(
                    "plugin '{}' updateUrl must start with http:// or https://",
                    self.id
                ));
            }
        }
        for permission in &self.permissions {
            if !permission.starts_with("net:") || permission.len() <= 4 {
                return Err(format!(
                    "invalid permission '{}' in plugin '{}': only 'net:<url-glob>' is supported",
                    permission, self.id
                ));
            }
        }
        Ok(())
    }

    /// 网络权限列表（net: 前缀的 glob 模式）。
    pub fn net_permission_patterns(&self) -> Vec<String> {
        self.permissions
            .iter()
            .filter_map(|p| p.strip_prefix("net:"))
            .map(|p| p.to_string())
            .collect()
    }
}
