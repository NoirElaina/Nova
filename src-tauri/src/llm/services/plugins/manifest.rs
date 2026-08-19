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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributes {
    #[serde(default)]
    pub tools: Vec<PluginToolSpec>,
    #[serde(default)]
    pub settings_tab: Option<PluginSettingsTab>,
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

impl PluginManifest {
    /// manifest 语义校验：插件 id / 工具名格式、权限声明格式。
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
