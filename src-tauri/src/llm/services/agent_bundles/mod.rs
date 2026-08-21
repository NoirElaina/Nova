// 智能体套件（Agent Bundle）：一个 bundle = 附加提示词 + 工具/技能/MCP 装备清单。
// 清单语义：null = 全部（跟随全局，含后续新增）；Some(清单) = 自定义勾选（勾=添加，不勾=移除）。
// 存储为目录 app_data/agents/<id>/（agent.json 定义 + skills/ 私有技能 + files/ 资料文件）；
// MCP 定义全局唯一（mcp_servers.json），智能体仅通过 enabled_mcp_servers 引用。
// 会话挂载的 bundle 记录在 conversations.active_agent_id。
// 智能体是会话级的：智能体页点「启用」只对当前对话生效，其他对话/新对话均为默认 Nova。
//
// 会话 -> bundle 的读取链路：DB 读取是异步的，而 provider adapter 的 build_request 是同步热路径，
// 故用「进程内缓存 + 写穿透」（与 workspace 缓存同模式）：
// - 写库（启用/移除/删除）后同步更新缓存；
// - list_conversations 批量刷新缓存；send_chat_message 轮次开始时对当前会话单刷兜底；
// - 同步读 active_bundle() 只查缓存，未命中返回 None（退化为默认 Nova，下一轮刷新即恢复）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// 这些工具是内核流程控制所必需的（计划模式切换、用户问答），
/// 不参与勾选清单——bundle 排除它们会破坏 agent 循环。
pub const ALWAYS_ON_TOOLS: &[&str] = &["EnterPlanMode", "ExitPlanMode", "ask_user_question"];

/// 仅默认 Nova 可用的工具：专用智能体不允许写全局记忆，防止领域会话污染跨会话记忆。
pub const DEFAULT_ONLY_TOOLS: &[&str] = &["memory"];

/// 智能体来源：手动创建 / 市场安装 / 导入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentSourceKind {
    #[default]
    Manual,
    Market,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentSource {
    #[serde(default)]
    pub kind: AgentSourceKind,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundle {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 附加提示词（markdown）。空 = 不追加。
    #[serde(default)]
    pub prompt: String,
    /// 工具清单。None = 全部；Some = 仅勾选的（流程控制工具恒可用）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// 技能清单。None = 全部已启用技能；Some = 仅勾选的。
    #[serde(default)]
    pub enabled_skills: Option<Vec<String>>,
    /// MCP server 引用清单（全局定义，仅引用不存储）。None = 全部；Some = 仅勾选的。
    #[serde(default)]
    pub enabled_mcp_servers: Option<Vec<String>>,
    /// 全局启用开关：禁用后不可被新会话挂载。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 来源标注（手动 / 市场 / 导入）。
    #[serde(default)]
    pub source: AgentSource,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_true() -> bool {
    true
}

/// 存量兼容：剔除 DEFAULT_ONLY_TOOLS（旧 bundle 可能勾选了 memory）。
fn sanitize_bundle(mut bundle: AgentBundle) -> AgentBundle {
    if let Some(tools) = bundle.enabled_tools.as_mut() {
        tools.retain(|name| {
            !DEFAULT_ONLY_TOOLS
                .iter()
                .any(|t| t.eq_ignore_ascii_case(name.trim()))
        });
    }
    bundle
}

// ---------------- 会话 -> bundle 进程内缓存（写穿透） ----------------

/// 会话id -> 挂载的 bundle id（None = 默认 Nova）。
static CONVERSATION_AGENT_CACHE: OnceLock<RwLock<HashMap<String, Option<String>>>> =
    OnceLock::new();

fn conversation_agent_cache() -> &'static RwLock<HashMap<String, Option<String>>> {
    CONVERSATION_AGENT_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 写穿透：更新单个会话的缓存条目。由 set_conversation_agent / list_conversations 调用。
pub fn cache_conversation_agent(conversation_id: &str, bundle_id: Option<&str>) {
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return;
    }
    if let Ok(mut cache) = conversation_agent_cache().write() {
        cache.insert(normalized.to_string(), bundle_id.map(|v| v.to_string()));
    }
}

/// 批量刷新缓存（先清后写）。由 list_conversations / 删除 bundle 清引用后调用。
pub fn refresh_conversation_agent_cache(entries: &[(String, Option<String>)]) {
    if let Ok(mut cache) = conversation_agent_cache().write() {
        cache.clear();
        for (id, bundle) in entries {
            cache.insert(id.clone(), bundle.clone());
        }
    }
}

/// 异步刷新单个会话的缓存（轮次开始时兜底，防止冷启动读不到）。
pub async fn refresh_single_conversation_agent(app: &AppHandle, conversation_id: &str) {
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return;
    }
    if let Ok(bundle_id) =
        crate::llm::history::get_conversation_agent(app, normalized).await
    {
        cache_conversation_agent(normalized, bundle_id.as_deref());
    }
}

/// 同步读取会话挂载的 bundle。缓存未命中返回 None（默认 Nova）。
/// provider adapter / system_prompt / SkillTool 等同步热路径一律走这里。
pub fn active_bundle(app: &AppHandle, conversation_id: Option<&str>) -> Option<AgentBundle> {
    let conv_id = conversation_id.map(str::trim).filter(|s| !s.is_empty())?;
    let bundle_id = conversation_agent_cache()
        .read()
        .ok()
        .and_then(|cache| cache.get(conv_id).cloned())
        .flatten()?;
    load_bundle(app, &bundle_id).ok()
}

// ---------------- bundle 文件存取 ----------------

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn bundles_root_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("agents"))
        .map_err(|e| format!("Failed to resolve app_data_dir for agents: {}", e))
}

fn ensure_bundles_root_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let root = bundles_root_dir(app)?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root)
}

/// bundle id 只允许安全字符，防路径穿越。
fn validate_bundle_id(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("bundle id is required".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("bundle id must not contain path separators".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("bundle id contains invalid characters".to_string());
    }
    Ok(())
}

fn bundle_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_bundle_id(id)?;
    let root = ensure_bundles_root_dir(app)?;
    Ok(root.join(id.trim()).join("agent.json"))
}

/// 智能体根目录：agents/<id>/。
pub fn agent_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_bundle_id(id)?;
    let root = ensure_bundles_root_dir(app)?;
    Ok(root.join(id.trim()))
}

/// 私有技能目录：agents/<id>/skills/。
pub fn agent_skills_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(agent_dir(app, id)?.join("skills"))
}

/// 资料文件目录：agents/<id>/files/。
pub fn agent_files_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(agent_dir(app, id)?.join("files"))
}

/// 旧版私有 MCP 配置文件：agents/<id>/mcp.json。
/// MCP 定义已全局唯一，此路径仅供启动时的存量迁移读取（迁入全局后改名 .migrated 备份）。
pub fn agent_mcp_config_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(agent_dir(app, id)?.join("mcp.json"))
}

/// 旧格式迁移：agents/<id>.json → agents/<id>/agent.json。
/// 读取/列表前调用；文件已迁移或不存在时静默跳过。
fn migrate_legacy_bundle_file(app: &AppHandle, root: &std::path::Path) {
    let entries = match std::fs::read_dir(root) {
        Ok(v) => v,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        if validate_bundle_id(stem).is_err() {
            continue;
        }
        let new_path = root.join(stem).join("agent.json");
        if new_path.exists() {
            // 目录里已有定义：旧文件视为废弃残留，直接删除防止反复回读。
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if std::fs::create_dir_all(root.join(stem)).is_ok() {
            if std::fs::rename(&path, &new_path).is_err() {
                // rename 失败（跨设备等）退回复制+删除。
                if std::fs::copy(&path, &new_path).is_ok() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    let _ = app; // 预留：迁移结果上报
}

/// 生成短 uuid（时间戳 + 随机后缀），无需额外依赖。
fn generate_bundle_id() -> String {
    let entropy = std::process::id() as u64 ^ now_unix_secs() as u64;
    let rand = (entropy.wrapping_mul(6364136223846793005)) >> 24 & 0xffffff;
    format!("agent-{:x}-{:06x}", now_unix_secs(), rand)
}

/// 归一化名字用于清单匹配（工具/技能/server 名大小写不敏感）。
fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

impl AgentBundle {
    /// 内置工具是否对该 bundle 可见（流程控制工具恒可见；默认专属工具恒不可见）。
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        if DEFAULT_ONLY_TOOLS.iter().any(|t| *t == tool_name) {
            return false;
        }
        if ALWAYS_ON_TOOLS.iter().any(|t| *t == tool_name) {
            return true;
        }
        match &self.enabled_tools {
            None => true,
            Some(allowed) => {
                allowed.iter().any(|n| normalize_name(n) == normalize_name(tool_name))
            }
        }
    }

    /// 技能是否对该 bundle 可见。
    pub fn is_skill_enabled(&self, skill_name: &str) -> bool {
        match &self.enabled_skills {
            None => true,
            Some(allowed) => {
                allowed.iter().any(|n| normalize_name(n) == normalize_name(skill_name))
            }
        }
    }

    /// MCP server 是否对该 bundle 可见。
    pub fn is_mcp_server_enabled(&self, server_name: &str) -> bool {
        match &self.enabled_mcp_servers {
            None => true,
            Some(allowed) => {
                allowed.iter().any(|n| normalize_name(n) == normalize_name(server_name))
            }
        }
    }
}

/// 列出全部 bundle（按更新时间倒序）。
pub fn list_bundles(app: &AppHandle) -> Result<Vec<AgentBundle>, String> {
    let root = ensure_bundles_root_dir(app)?;
    // 旧单文件格式先迁移到目录格式。
    migrate_legacy_bundle_file(app, &root);

    let mut items = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        // 目录制：agents/<id>/agent.json。
        if !path.is_dir() {
            continue;
        }
        let definition = path.join("agent.json");
        if !definition.is_file() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&definition) {
            if let Ok(bundle) = serde_json::from_str::<AgentBundle>(&content) {
                items.push(sanitize_bundle(bundle));
            }
        }
    }
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.name.cmp(&b.name)));
    Ok(items)
}

/// 读取单个 bundle。
pub fn load_bundle(app: &AppHandle, id: &str) -> Result<AgentBundle, String> {
    let path = bundle_path(app, id)?;
    if !path.exists() {
        // 可能是旧格式还没迁移（load 未经过 list_bundles），先迁移再读一次。
        if let Ok(root) = bundles_root_dir(app) {
            migrate_legacy_bundle_file(app, &root);
        }
        if !path.exists() {
            return Err(format!("Agent bundle not found: {}", id));
        }
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let bundle: AgentBundle =
        serde_json::from_str(&content).map_err(|e| format!("Invalid agent bundle {}: {}", id, e))?;
    Ok(sanitize_bundle(bundle))
}

/// 保存 bundle（整体写入），并确保目录骨架（skills/、files/）存在。
pub fn save_bundle(app: &AppHandle, bundle: AgentBundle) -> Result<AgentBundle, String> {
    if bundle.name.trim().is_empty() {
        return Err("bundle name is required".to_string());
    }
    let mut bundle = bundle;
    bundle.name = bundle.name.trim().to_string();
    bundle.updated_at = now_unix_secs();

    let path = bundle_path(app, &bundle.id)?;
    // 新建时补 createdAt；已存在的保留原值。
    if !path.exists() && bundle.created_at <= 0 {
        bundle.created_at = bundle.updated_at;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // 目录骨架：私有技能与资料目录随保存一并确保。
        for sub in ["skills", "files"] {
            std::fs::create_dir_all(parent.join(sub)).map_err(|e| e.to_string())?;
        }
    }
    let content = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(bundle)
}

/// 创建空白 bundle（默认全部能力满配，等用户做加减法）。
pub fn create_bundle(app: &AppHandle, name: &str) -> Result<AgentBundle, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("bundle name is required".to_string());
    }
    let now = now_unix_secs();
    let bundle = AgentBundle {
        id: generate_bundle_id(),
        name: trimmed.to_string(),
        description: String::new(),
        prompt: String::new(),
        enabled_tools: None,
        enabled_skills: None,
        enabled_mcp_servers: None,
        enabled: true,
        source: AgentSource::default(),
        created_at: now,
        updated_at: now,
    };
    save_bundle(app, bundle)
}

/// 删除 bundle（整个目录：定义 + 私有技能 + 资料），
/// 并同步清空所有挂载它的会话引用（回到默认 Nova）+ 刷新缓存。
pub async fn delete_bundle(app: &AppHandle, id: &str) -> Result<(), String> {
    let dir = agent_dir(app, id)?;
    if !dir.exists() {
        return Err(format!("Agent bundle not found: {}", id));
    }
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    crate::llm::history::clear_conversation_agent_references(app, id).await?;
    Ok(())
}
