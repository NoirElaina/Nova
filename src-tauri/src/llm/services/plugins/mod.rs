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
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

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
    pub commands: Vec<manifest::PluginCommandSpec>,
    pub prompt_section: Option<manifest::PluginPromptSection>,
    /// 系统内置插件：不可卸载，只可停用。
    pub system: bool,
    /// 更新检查源（指向新版本 zip 的 URL）。
    pub update_url: Option<String>,
    pub enabled: bool,
    pub dir: String,
    /// manifest 解析或校验失败时的错误信息（此时其余字段为降级值）。
    pub error: Option<String>,
}

impl PluginInfo {
    /// 解析失败插件的降级构造（保持字段齐全）。
    fn broken(id: String, dir: String, error: String) -> Self {
        Self {
            id: id.clone(),
            name: id,
            version: String::new(),
            description: String::new(),
            author: String::new(),
            permissions: Vec::new(),
            tools: Vec::new(),
            settings_tab: None,
            commands: Vec::new(),
            prompt_section: None,
            system: false,
            update_url: None,
            enabled: false,
            dir,
            error: Some(error),
        }
    }
}

/// 前端斜杠命令列表条目（IPC 返回结构）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommandInfo {
    pub plugin_id: String,
    pub plugin_name: String,
    pub name: String,
    pub title: String,
    pub description: String,
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
                broken.push(PluginInfo::broken(
                    folder_name,
                    dir.to_string_lossy().to_string(),
                    error,
                ));
            }
        }
    }

    // 构建工具与命令分发索引：拒绝与内置工具/命令或其它插件重名。
    let builtin = builtin_tool_names();
    let mut tool_owner: HashMap<String, String> = HashMap::new();
    let mut command_owner: HashMap<String, String> = HashMap::new();
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
        if conflict.is_none() {
            for command in &entry.manifest.contributes.commands {
                if let Some(owner) = command_owner.get(&command.name) {
                    conflict = Some(format!(
                        "command '/{}' conflicts with plugin '{}'",
                        command.name, owner
                    ));
                    break;
                }
                command_owner.insert(command.name.clone(), entry.manifest.id.clone());
            }
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
                commands: entry.manifest.contributes.commands,
                prompt_section: entry.manifest.contributes.prompt_section,
                system: entry.manifest.system,
                update_url: entry.manifest.update_url,
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
                commands: entry.manifest.contributes.commands.clone(),
                prompt_section: entry.manifest.contributes.prompt_section.clone(),
                system: entry.manifest.system,
                update_url: entry.manifest.update_url.clone(),
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
    conversation_id: Option<&str>,
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
        .load(
            &plugin_id,
            code,
            read_plugin_settings(app, &plugin_id),
            &manifest,
            app.clone(),
        )
        .await
    {
        return Some(Err(ToolFailure::new(format!(
            "plugin '{}' failed to load: {}",
            plugin_id, error
        ))));
    }

    let result = sandbox_runtime()
        .call(&plugin_id, name, input, conversation_id.map(str::to_string))
        .await;
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
    // UI 桥无会话上下文，conversation_id 传 None。
    match execute_plugin_tool(app, None, tool, args).await {
        Some(Ok(outcome)) => match serde_json::from_str::<Json>(&outcome.output) {
            Ok(value) => Ok(value),
            // 非 JSON 文本输出时包装为 {result: text}，保持桥协议稳定。
            Err(_) => Ok(serde_json::json!({ "result": outcome.output })),
        },
        Some(Err(failure)) => Err(failure.message),
        None => Err(format!("tool '{}' not found", tool)),
    }
}

// ---------------- 贡献点提取（命令 / 提示词片段） ----------------

/// 当前启用插件的斜杠命令清单（前端命令列表合并用）。
pub fn plugin_commands(app: &AppHandle) -> Vec<PluginCommandInfo> {
    let needs_scan = registry()
        .lock()
        .map(|state| !state.refreshed)
        .unwrap_or(true);
    if needs_scan {
        refresh_registry(app);
    }
    let mut commands = Vec::new();
    if let Ok(state) = registry().lock() {
        for entry in state.entries.iter().filter(|entry| entry.enabled) {
            for command in &entry.manifest.contributes.commands {
                commands.push(PluginCommandInfo {
                    plugin_id: entry.manifest.id.clone(),
                    plugin_name: entry.manifest.name.clone(),
                    name: command.name.clone(),
                    title: command.title.clone(),
                    description: command.description.clone(),
                });
            }
        }
    }
    commands
}

/// 展开插件命令的 promptTemplate：替换 {workspace} / {date} 占位符。
pub fn expand_plugin_command(
    app: &AppHandle,
    plugin_id: &str,
    name: &str,
    conversation_id: Option<&str>,
) -> Result<String, String> {
    let state = registry()
        .lock()
        .map_err(|_| "plugin registry lock poisoned".to_string())?;
    let entry = state
        .entries
        .iter()
        .find(|entry| entry.manifest.id == plugin_id)
        .ok_or_else(|| format!("unknown plugin '{}'", plugin_id))?;
    if !entry.enabled {
        return Err(format!("plugin '{}' is disabled", plugin_id));
    }
    let command = entry
        .manifest
        .contributes
        .commands
        .iter()
        .find(|command| command.name == name)
        .ok_or_else(|| format!("plugin '{}' has no command '{}'", plugin_id, name))?;

    let workspace = crate::command::workspace::workspace_root_for_conversation(
        app,
        conversation_id,
    )
    .map(|path| path.display().to_string())
    .unwrap_or_default();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    Ok(command
        .prompt_template
        .replace("{workspace}", &workspace)
        .replace("{date}", &today))
}

/// 当前启用插件的提示词片段（系统提示词拼接用）。
/// 返回 (插件名, 片段内容, 锚点) 列表。
pub fn plugin_prompt_sections(app: &AppHandle) -> Vec<(String, String, String)> {
    let needs_scan = registry()
        .lock()
        .map(|state| !state.refreshed)
        .unwrap_or(true);
    if needs_scan {
        refresh_registry(app);
    }
    let mut sections = Vec::new();
    if let Ok(state) = registry().lock() {
        for entry in state.entries.iter().filter(|entry| entry.enabled) {
            if let Some(section) = &entry.manifest.contributes.prompt_section {
                sections.push((
                    entry.manifest.name.clone(),
                    section.content.trim().to_string(),
                    section.placement.clone(),
                ));
            }
        }
    }
    sections
}

// ---------------- 生命周期：卸载 / 安装 / 更新 ----------------

/// 卸载插件：卸载沙箱槽位、删除插件目录与私有设置、刷新注册表。
pub fn uninstall_plugin(app: &AppHandle, plugin_id: &str) -> Result<(), String> {
    let root = plugins_root(app);
    let target = root.join(plugin_id);
    if !target.exists() {
        return Err(format!("plugin '{}' is not installed", plugin_id));
    }
    // 路径安全：plugin_id 必须是纯目录名（防穿越）。
    if plugin_id.is_empty()
        || plugin_id.contains('/')
        || plugin_id.contains('\\')
        || plugin_id.contains("..")
        || plugin_id.starts_with('.')
    {
        return Err(format!("invalid plugin id '{}'", plugin_id));
    }

    // 读取 manifest 检查系统插件标记。
    let manifest_path = target.join("plugin.json");
    if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = serde_json::from_str::<PluginManifest>(&content) {
            if manifest.system {
                return Err(format!(
                    "plugin '{}' is a system plugin and cannot be uninstalled (disable it instead)",
                    plugin_id
                ));
            }
        }
    }

    sandbox_runtime().unload(plugin_id);

    std::fs::remove_dir_all(&target)
        .map_err(|e| format!("failed to remove plugin dir '{}': {}", target.display(), e))?;
    let settings = settings_file(app, plugin_id);
    if settings.exists() {
        let _ = std::fs::remove_file(&settings);
    }

    // 从停用清单移除（残留无害但清理干净）。
    if let Ok(mut settings) = crate::command::settings::get_settings(app.clone()) {
        let before = settings.disabled_plugins.len();
        settings
            .disabled_plugins
            .retain(|id| id != plugin_id);
        if settings.disabled_plugins.len() != before {
            let _ = crate::command::settings::save_settings(app.clone(), settings);
        }
    }

    refresh_registry(app);
    info!(plugin = %plugin_id, "plugin uninstalled");
    Ok(())
}

/// zip 安装上限：防 zip 炸弹。
const INSTALL_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const INSTALL_MAX_FILE_COUNT: usize = 2000;

/// 从 zip 安装/覆盖更新插件：
/// 解压到临时目录 → 校验 plugin.json → 移入 plugins/<id>/（覆盖时保留 .settings）→ 刷新。
pub async fn install_plugin_from_zip(app: &AppHandle, zip_path: &str) -> Result<PluginInfo, String> {
    let zip_file = std::fs::File::open(zip_path)
        .map_err(|e| format!("failed to open plugin zip '{}': {}", zip_path, e))?;
    let mut archive =
        zip::ZipArchive::new(zip_file).map_err(|e| format!("invalid plugin zip: {}", e))?;

    // 安全校验：条目路径拒绝穿越/绝对路径，统计总大小防 zip 炸弹。
    let mut total_bytes: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("failed to read zip entry: {}", e))?;
        if index >= INSTALL_MAX_FILE_COUNT {
            return Err("plugin zip contains too many files (>2000)".to_string());
        }
        let name = entry.name().to_string();
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') || name.contains('\\') {
            return Err(format!("plugin zip contains unsafe path '{}'", name));
        }
        total_bytes += entry.size();
        if total_bytes > INSTALL_MAX_TOTAL_BYTES {
            return Err("plugin zip is too large (>64MB after extract)".to_string());
        }
    }

    // 解压到临时目录。
    let tmp_root = std::env::temp_dir().join(format!(
        "nova-plugin-install-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&tmp_root)
        .map_err(|e| format!("failed to create temp dir: {}", e))?;
    archive
        .extract(&tmp_root)
        .map_err(|e| format!("failed to extract plugin zip: {}", e))?;

    // 定位 plugin.json：解压根目录直接有，或根下唯一一层子目录内有。
    let mut source_dir = tmp_root.clone();
    if !source_dir.join("plugin.json").exists() {
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(&tmp_root)
            .map_err(|e| format!("failed to scan extracted dir: {}", e))?
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.path())
            .filter(|path| path.join("plugin.json").exists())
            .collect();
        if candidates.len() != 1 {
            let _ = std::fs::remove_dir_all(&tmp_root);
            return Err(
                "plugin zip must contain plugin.json at the root (or exactly one subfolder with it)"
                    .to_string(),
            );
        }
        source_dir = candidates.remove(0);
    }

    // 解析并校验 manifest。
    let manifest_content = std::fs::read_to_string(source_dir.join("plugin.json"))
        .map_err(|e| format!("failed to read plugin.json: {}", e))?;
    let manifest: PluginManifest = serde_json::from_str(&manifest_content)
        .map_err(|e| format!("failed to parse plugin.json: {}", e))?;
    manifest
        .validate()
        .map_err(|e| format!("plugin.json validation failed: {}", e))?;

    // 移入正式目录（同 id = 覆盖更新，.settings 独立目录自动保留）。
    let root = plugins_root(app);
    std::fs::create_dir_all(&root).map_err(|e| format!("failed to create plugins dir: {}", e))?;
    let target = root.join(&manifest.id);
    if target.exists() {
        // 卸载沙箱槽位强制下次懒加载新代码。
        sandbox_runtime().unload(&manifest.id);
        std::fs::remove_dir_all(&target).map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp_root);
            format!("failed to replace old plugin dir: {}", e)
        })?;
    }
    std::fs::rename(&source_dir, &target).map_err(|e| {
        let _ = std::fs::remove_dir_all(&tmp_root);
        format!(
            "failed to move plugin into '{}': {}",
            target.display(),
            e
        )
    })?;
    let _ = std::fs::remove_dir_all(&tmp_root);

    // 刷新并返回新插件信息。
    let broken = refresh_registry(app);
    if let Some(info) = broken.into_iter().find(|info| info.id == manifest.id) {
        return Err(info.error.unwrap_or_else(|| "plugin loaded with errors".into()));
    }
    list_plugins(app)
        .into_iter()
        .find(|info| info.id == manifest.id && info.error.is_none())
        .ok_or_else(|| "plugin installed but not found in registry".to_string())
}

/// 语义化版本比较：返回 Some(true) 表示 remote 比 current 新。
fn version_gt(remote: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|part| {
                part.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    };
    let remote_parts = parse(remote);
    let current_parts = parse(current);
    for i in 0..remote_parts.len().max(current_parts.len()) {
        let r = remote_parts.get(i).copied().unwrap_or(0);
        let c = current_parts.get(i).copied().unwrap_or(0);
        if r != c {
            return r > c;
        }
    }
    false
}

/// 检查插件更新：下载 updateUrl 指向的 zip，解析其中 manifest 的版本号并比较。
/// 返回 { hasUpdate, currentVersion, remoteVersion }。
pub async fn check_plugin_update(_app: &AppHandle, plugin_id: &str) -> Result<Json, String> {
    let (current_version, update_url) = {
        let state = registry()
            .lock()
            .map_err(|_| "plugin registry lock poisoned".to_string())?;
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.manifest.id == plugin_id)
            .ok_or_else(|| format!("unknown plugin '{}'", plugin_id))?;
        (
            entry.manifest.version.clone(),
            entry.manifest
                .update_url
                .clone()
                .ok_or_else(|| format!("plugin '{}' does not declare updateUrl", plugin_id))?,
        )
    };

    // 下载远端 zip 到临时文件。
    let client = reqwest::Client::builder()
        .timeout(HTTP_UPDATE_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(&update_url)
        .send()
        .await
        .map_err(|e| format!("failed to download update: {}", e))?
        .error_for_status()
        .map_err(|e| format!("update download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("failed to read update body: {}", e))?;
    let tmp_zip = std::env::temp_dir().join(format!(
        "nova-plugin-update-{}.zip",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp_zip, &bytes)
        .map_err(|e| format!("failed to write temp zip: {}", e))?;

    // 解析 zip 内 plugin.json 的版本号（只读不安装）。
    let file = std::fs::File::open(&tmp_zip)
        .map_err(|e| format!("failed to open downloaded zip: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("invalid downloaded zip: {}", e))?;
    let remote_version = find_manifest_in_zip(&mut archive)?
        .ok_or_else(|| "downloaded zip has no plugin.json".to_string())?
        .version;
    let _ = std::fs::remove_file(&tmp_zip);

    Ok(serde_json::json!({
        "hasUpdate": version_gt(&remote_version, &current_version),
        "currentVersion": current_version,
        "remoteVersion": remote_version,
    }))
}

/// 从 zip 中提取 manifest（根目录优先，回退唯一一层子目录）。
fn find_manifest_in_zip(
    archive: &mut zip::ZipArchive<std::fs::File>,
) -> Result<Option<PluginManifest>, String> {
    // 先收集候选条目名：根目录 plugin.json 优先，其次一层子目录内的。
    let mut root_manifest: Option<String> = None;
    let mut subdir_manifest: Option<String> = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("failed to read zip entry: {}", e))?;
        let name = entry.name().to_string();
        if name == "plugin.json" {
            root_manifest = Some(name);
            break;
        }
        if name.ends_with("/plugin.json") && name.matches('/').count() == 1 && subdir_manifest.is_none() {
            subdir_manifest = Some(name);
        }
    }
    let target = root_manifest.or(subdir_manifest);
    let Some(entry_name) = target else {
        return Ok(None);
    };

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("failed to read zip entry: {}", e))?;
        if entry.name() != entry_name {
            continue;
        }
        let mut content = String::new();
        std::io::Read::read_to_string(&mut entry, &mut content)
            .map_err(|e| format!("failed to read plugin.json in zip: {}", e))?;
        let manifest: PluginManifest = serde_json::from_str(&content)
            .map_err(|e| format!("failed to parse plugin.json in zip: {}", e))?;
        return Ok(Some(manifest));
    }
    Ok(None)
}

const HTTP_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// 应用插件更新：从 updateUrl 下载并走安装管线（覆盖，保留 .settings）。
pub async fn update_plugin(app: &AppHandle, plugin_id: &str) -> Result<(), String> {
    let update_url = {
        let state = registry()
            .lock()
            .map_err(|_| "plugin registry lock poisoned".to_string())?;
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.manifest.id == plugin_id)
            .ok_or_else(|| format!("unknown plugin '{}'", plugin_id))?;
        entry
            .manifest
            .update_url
            .clone()
            .ok_or_else(|| format!("plugin '{}' does not declare updateUrl", plugin_id))?
    };

    let client = reqwest::Client::builder()
        .timeout(HTTP_UPDATE_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(&update_url)
        .send()
        .await
        .map_err(|e| format!("failed to download update: {}", e))?
        .error_for_status()
        .map_err(|e| format!("update download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("failed to read update body: {}", e))?;
    let tmp_zip = std::env::temp_dir().join(format!(
        "nova-plugin-update-{}.zip",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp_zip, &bytes)
        .map_err(|e| format!("failed to write temp zip: {}", e))?;

    let result = install_plugin_from_zip(app, tmp_zip.to_string_lossy().as_ref()).await;
    let _ = std::fs::remove_file(&tmp_zip);
    result.map(|_| ())
}

// ---------------- 插件目录监听（notify） ----------------

/// 启动 plugins/ 目录监听：文件变化自动刷新注册表、卸载受影响沙箱槽位并推送前端事件。
/// 开发插件时改 main.js 免重启——下次工具调用懒加载新代码。
pub fn start_plugins_watcher(app: AppHandle) {
    use notify::Watcher;
    use std::sync::mpsc::RecvTimeoutError;

    std::thread::Builder::new()
        .name("nova-plugin-watcher".to_string())
        .spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let Ok(mut watcher) = notify::recommended_watcher(tx) else {
                warn!("failed to create plugin directory watcher");
                return;
            };
            let root = plugins_root(&app);
            if std::fs::create_dir_all(&root).is_err() {
                warn!("failed to create plugins dir for watcher");
                return;
            }
            if let Err(error) = watcher.watch(&root, notify::RecursiveMode::Recursive) {
                warn!(error = %error, "failed to watch plugins dir");
                return;
            }
            info!("plugin directory watcher started");

            // 防抖：事件静默 800ms 后统一处理，期间收集涉及的插件 id。
            let mut pending_ids: Vec<String> = Vec::new();
            loop {
                match rx.recv_timeout(std::time::Duration::from_millis(800)) {
                    Ok(event) => {
                        if let Ok(notify::Event { paths, .. }) = event {
                            for path in paths {
                                if let Some(id) = plugin_id_from_path(&root, &path) {
                                    if !pending_ids.contains(&id) {
                                        pending_ids.push(id);
                                    }
                                }
                            }
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if pending_ids.is_empty() {
                            continue;
                        }
                        let ids = std::mem::take(&mut pending_ids);
                        info!(plugins = ?ids, "plugins dir changed, refreshing registry");
                        // 变更插件的沙箱槽位全部卸载：main.js 可能已改，
                        // 下次工具调用会懒加载新代码（开发即时生效）。
                        for id in &ids {
                            sandbox_runtime().unload(id);
                        }
                        refresh_registry(&app);
                        let _ = app.emit("plugins-changed", ());
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .ok();
}

/// 从监听事件路径提取插件 id（plugins/<id>/... 形式）。
fn plugin_id_from_path(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let first = relative.components().next()?;
    let name = first.as_os_str().to_string_lossy().to_string();
    if name.is_empty() || name.starts_with('.') {
        return None;
    }
    Some(name)
}
