// 智能体套件（Agent Bundle）：一个 bundle = 附加提示词 + 工具/技能/MCP 装备清单。
// 清单语义：null = 全部（跟随全局，含后续新增）；Some(清单) = 自定义勾选（勾=添加，不勾=移除）。
// 存储为 app_data/agents/<id>.json；会话挂载的 bundle 记录在 conversations.active_agent_id。
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
    /// MCP server 清单。None = 全部已连接服务器；Some = 仅勾选的。
    #[serde(default)]
    pub enabled_mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
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
    Ok(root.join(format!("{}.json", id.trim())))
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
    /// 内置工具是否对该 bundle 可见（流程控制工具恒可见）。
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
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
    let mut items = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(bundle) = serde_json::from_str::<AgentBundle>(&content) {
                items.push(bundle);
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
        return Err(format!("Agent bundle not found: {}", id));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| format!("Invalid agent bundle {}: {}", id, e))
}

/// 保存 bundle（整体写入）。
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
        created_at: now,
        updated_at: now,
    };
    save_bundle(app, bundle)
}

/// 删除 bundle 文件。会话引用清理由异步命令层负责
/// （UPDATE conversations + 刷新缓存），见 command::agent_config::delete_agent_bundle。
pub fn delete_bundle(app: &AppHandle, id: &str) -> Result<(), String> {
    let path = bundle_path(app, id)?;
    if !path.exists() {
        return Err(format!("Agent bundle not found: {}", id));
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}
