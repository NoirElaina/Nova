// 插件系统服务：发现、注册表缓存、工具目录合并、工具分发、设置持久化。
//
// 目录布局（app_data_dir/plugins/）：
//   plugins/<plugin-id>/plugin.json   清单（工具/界面/权限声明）
//   plugins/<plugin-id>/main.js       沙箱入口（注册工具实现）
//   plugins/<plugin-id>/ui/...        界面文件（经 nova-plugin:// 协议供给 iframe）
//   plugins/.settings/<plugin-id>.json 插件私有设置（独立于插件目录，升级插件不丢配置）
//
// 注册表缓存（REGISTRY）是同步可读的静态视图，供每次 LLM 请求组装工具列表；
// 沙箱执行按需懒加载（首次调用工具时 load main.js）。

pub mod manifest;
mod sandbox;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value as Json;
use tauri::{AppHandle, Manager};
use tracing::warn;

use crate::llm::tools::{builtin_tool_names, ToolExecResult, ToolFailure, ToolOutcome};
use crate::llm::types::Tool;

use manifest::{PluginManifest, PluginToolSpec};
use sandbox::sandbox as sandbox_runtime;

/// 设置页展示用插件信息（IPC 返回结构）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub permissions: Vec<String>,
    pub tools: Vec<PluginToolSpec>,
    pub settings_tab: Option<manifest::PluginSettingsTab>,
    pub enabled: bool,
    pub dir: String,
    /// manifest 解析或校验失败时的错误信息（此时其余字段为降级值）。
    pub error: Option<String>,
}

/// 注册表条目：解析成功的插件。
struct RegistryEntry {
    manifest: PluginManifest,
    dir: PathBuf,
    enabled: bool,
}

#[derive(Default)]
struct RegistryState {
    entries: Vec<RegistryEntry>,
    /// 工具名 → 插件 id 的分发索引（跨插件重名在校验阶段拒绝）。
    tool_owner: HashMap<String, String>,
    refreshed: bool,
}

fn registry() -> &'static Arc<Mutex<RegistryState>> {
    static REGISTRY: OnceLock<Arc<Mutex<RegistryState>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(RegistryState::default())))
}

pub fn plugins_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("plugins")
}

fn settings_dir(app: &AppHandle) -> PathBuf {
    plugins_root(app).join(".settings")
}

fn settings_file(app: &AppHandle, plugin_id: &str) -> PathBuf {
    settings_dir(app).join(format!("{}.json", plugin_id))
}

/// 读取插件设置（不存在时返回空对象）。
pub fn read_plugin_settings(app: &AppHandle, plugin_id: &str) -> Json {
    let path = settings_file(app, plugin_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|error| {
            warn!(plugin = %plugin_id, error = %error, "invalid plugin settings file, fallback to empty");
            serde_json::json!({})
        }),
        Err(_) => serde_json::json!({}),
    }
}

/// 写入插件设置并同步到沙箱内存快照。
pub fn write_plugin_settings(app: &AppHandle, plugin_id: &str, settings: &Json) -> Result<(), String> {
    let dir = settings_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create plugin settings dir: {}", e))?;
    let content = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("failed to serialize plugin settings: {}", e))?;
    atomic_write(&settings_file(app, plugin_id), &content)?;
    sandbox_runtime().update_settings(plugin_id, settings.clone());
    Ok(())
}

fn atomic_write(path: &PathBuf, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("failed to write plugin settings: {}", e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("failed to replace plugin settings: {}", e))?;
    Ok(())
}

/// 扫描插件目录并重建注册表缓存。返回解析失败的插件（供设置页展示错误）。
pub fn refresh_registry(app: &AppHandle) -> Vec<PluginInfo> {
    let root = plugins_root(app);
    let mut entries = Vec::new();
    let mut broken = Vec::new();
    let disabled = disabled_plugin_ids(app);

    // 目录不存在（尚未安装任何插件）时返回空列表而非报错。
    let scan: Vec<std::fs::DirEntry> = match std::fs::read_dir(&root) {
        Ok(iter) => iter.flatten().collect(),
        Err(_) => Vec::new(),
    };

    for entry in scan {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let folder_name = entry.file_name().to_string_lossy().to_string();
        if folder_name.starts_with('.') {
            continue;
        }

        let manifest_path = dir.join("plugin.json");
        let parsed: Result<PluginManifest, String> = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("failed to read plugin.json: {}", e))
            .and_then(|content| {
                serde_json::from_str::<PluginManifest>(&content)
                    .map_err(|e| format!("failed to parse plugin.json: {}", e))
            })
            .and_then(|manifest| {
                manifest
                    .validate()
                    .map_err(|e| format!("plugin.json validation failed: {}", e))?;
                // 插件 id 必须与目录名一致，防止目录伪装。
                if manifest.id != folder_name {
                    return Err(format!(
                        "plugin id '{}' does not match folder name '{}'",
                        manifest.id, folder_name
                    ));
                }
                Ok(manifest)
            });

        match parsed {
            Ok(manifest) => {
                let enabled = !disabled.contains(&manifest.id);
                entries.push(RegistryEntry {
                    manifest,
                    dir,
                    enabled,
                });
            }
            Err(error) => {
                broken.push(PluginInfo {
                    id: folder_name.clone(),
                    name: folder_name,
                    version: String::new(),
                    description: String::new(),
                    author: String::new(),
                    permissions: Vec::new(),
                    tools: Vec::new(),
                    settings_tab: None,
                    enabled: false,
                    dir: dir.to_string_lossy().to_string(),
                    error: Some(error),
                });
            }
        }
    }

    // 构建工具分发索引：拒绝与内置工具或其它插件工具重名。
    let builtin = builtin_tool_names();
    let mut tool_owner: HashMap<String, String> = HashMap::new();
    let mut valid_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut conflict: Option<String> = None;
        for tool in &entry.manifest.contributes.tools {
            if builtin.contains(&tool.name) {
                conflict = Some(format!(
                    "tool '{}' conflicts with a builtin tool name",
                    tool.name
                ));
                break;
            }
            if let Some(owner) = tool_owner.get(&tool.name) {
                conflict = Some(format!(
                    "tool '{}' conflicts with plugin '{}'",
                    tool.name, owner
                ));
                break;
            }
            tool_owner.insert(tool.name.clone(), entry.manifest.id.clone());
        }

        match conflict {
            Some(error) => broken.push(PluginInfo {
                id: entry.manifest.id,
                name: entry.manifest.name,
                version: entry.manifest.version,
                description: entry.manifest.description,
                author: entry.manifest.author,
                permissions: entry.manifest.permissions,
                tools: entry.manifest.contributes.tools,
                settings_tab: entry.manifest.contributes.settings_tab,
                enabled: false,
                dir: entry.dir.to_string_lossy().to_string(),
                error: Some(error),
            }),
            None => valid_entries.push(entry),
        }
    }

    if let Ok(mut state) = registry().lock() {
        state.entries = valid_entries;
        state.tool_owner = tool_owner;
        state.refreshed = true;
    }
    broken
}

fn disabled_plugin_ids(app: &AppHandle) -> Vec<String> {
    crate::command::settings::get_settings(app.clone())
        .map(|settings| settings.disabled_plugins)
        .unwrap_or_default()
}

/// 当前注册表（含损坏插件的展示信息）。
pub fn list_plugins(app: &AppHandle) -> Vec<PluginInfo> {
    let broken = refresh_registry(app);
    let mut infos = broken;
    if let Ok(state) = registry().lock() {
        for entry in &state.entries {
            infos.push(PluginInfo {
                id: entry.manifest.id.clone(),
                name: entry.manifest.name.clone(),
                version: entry.manifest.version.clone(),
                description: entry.manifest.description.clone(),
                author: entry.manifest.author.clone(),
                permissions: entry.manifest.permissions.clone(),
                tools: entry.manifest.contributes.tools.clone(),
                settings_tab: entry.manifest.contributes.settings_tab.clone(),
                enabled: entry.enabled,
                dir: entry.dir.to_string_lossy().to_string(),
                error: None,
            });
        }
    }
    infos.sort_by(|a, b| a.id.cmp(&b.id));
    infos
}

/// 启用/停用插件：更新设置、刷新注册表、卸载沙箱槽位。
pub fn set_plugin_enabled(app: &AppHandle, plugin_id: &str, enabled: bool) -> Result<(), String> {
    let mut settings = crate::command::settings::get_settings(app.clone())?;
    let disabled: Vec<String> = settings
        .disabled_plugins
        .into_iter()
        .filter(|id| id != plugin_id)
        .collect();
    settings.disabled_plugins = if enabled {
        disabled
    } else {
        let mut next = disabled;
        next.push(plugin_id.to_string());
        next
    };
    crate::command::settings::save_settings(app.clone(), settings)?;
    refresh_registry(app);
    if !enabled {
        sandbox_runtime().unload(plugin_id);
    }
    Ok(())
}

/// 当前启用插件的工具目录（同步读取缓存，供每次 LLM 请求合并）。
pub fn plugin_tools(app: &AppHandle) -> Vec<Tool> {
    // 首次访问（例如启动后未打开过插件页）时补一次扫描。
    let needs_scan = registry()
        .lock()
        .map(|state| !state.refreshed)
        .unwrap_or(true);
    if needs_scan {
        refresh_registry(app);
    }

    let mut tools = Vec::new();
    if let Ok(state) = registry().lock() {
        for entry in state.entries.iter().filter(|entry| entry.enabled) {
            for spec in &entry.manifest.contributes.tools {
                tools.push(Tool {
                    name: spec.name.clone(),
                    description: format!(
                        "[plugin:{}] {}",
                        entry.manifest.id, spec.description
                    ),
                    input_schema: spec.parameters.clone(),
                });
            }
        }
    }
    tools
}

/// 工具名是否属于某个已启用插件。
fn plugin_id_for_tool(tool_name: &str) -> Option<String> {
    registry()
        .lock()
        .ok()
        .and_then(|state| state.tool_owner.get(tool_name).cloned())
}

/// 插件工具执行入口：按需懒加载沙箱槽位，校验入参 schema 后调用 JS handler。
/// 返回 None 表示该工具名不属于插件（交回内置/MCP 分发链）。
pub(crate) async fn execute_plugin_tool(
    app: &AppHandle,
    name: &str,
    input: Json,
) -> Option<ToolExecResult> {
    let plugin_id = plugin_id_for_tool(name)?;

    let (spec, manifest, dir) = {
        let state = registry().lock().ok()?;
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.manifest.id == plugin_id)?;
        if !entry.enabled {
            return Some(Err(ToolFailure::new(format!(
                "plugin '{}' is disabled",
                plugin_id
            ))));
        }
        let spec = entry
            .manifest
            .contributes
            .tools
            .iter()
            .find(|spec| spec.name == name)?
            .clone();
        (spec, entry.manifest.clone(), entry.dir.clone())
    };

    // 入参 schema 校验（与内置工具同等强度的 fail-fast）。
    if let Ok(validator) = jsonschema::validator_for(&spec.parameters) {
        let errors: Vec<_> = validator.iter_errors(&input).collect();
        if !errors.is_empty() {
            let detail = errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Some(Err(ToolFailure::invalid_input(format!(
                "Input validation failed for '{}': {}",
                name, detail
            ))));
        }
    }

    // 懒加载：读取 main.js + 设置快照，注入沙箱。
    let main_path = dir.join("main.js");
    let code = match std::fs::read_to_string(&main_path) {
        Ok(code) => code,
        Err(error) => {
            return Some(Err(ToolFailure::new(format!(
                "plugin '{}' main.js unreadable: {}",
                plugin_id, error
            ))))
        }
    };
    if let Err(error) = sandbox_runtime()
        .load(&plugin_id, code, read_plugin_settings(app, &plugin_id), &manifest)
        .await
    {
        return Some(Err(ToolFailure::new(format!(
            "plugin '{}' failed to load: {}",
            plugin_id, error
        ))));
    }

    let result = sandbox_runtime().call(&plugin_id, name, input).await;
    Some(match result {
        Ok(value) => Ok(ToolOutcome::json(value)),
        Err(error) => Err(ToolFailure::new(error)),
    })
}

/// 插件 UI 调用自家工具的直通入口（设置页桥 nova:callTool 使用）。
pub async fn call_plugin_tool_direct(
    app: &AppHandle,
    plugin_id: &str,
    tool: &str,
    args: Json,
) -> Result<Json, String> {
    // 确保该工具确实属于此插件，防止 UI 桥越权调用其它插件/内置工具。
    let owner = plugin_id_for_tool(tool);
    if owner.as_deref() != Some(plugin_id) {
        return Err(format!("tool '{}' does not belong to plugin '{}'", tool, plugin_id));
    }
    // 借用执行链完成懒加载与校验，但丢弃其结果包装，直接取 JSON。
    match execute_plugin_tool(app, tool, args).await {
        Some(Ok(outcome)) => match serde_json::from_str::<Json>(&outcome.output) {
            Ok(value) => Ok(value),
            // 非 JSON 文本输出时包装为 {result: text}，保持桥协议稳定。
            Err(_) => Ok(serde_json::json!({ "result": outcome.output })),
        },
        Some(Err(failure)) => Err(failure.message),
        None => Err(format!("tool '{}' not found", tool)),
    }
}
