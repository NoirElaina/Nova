use crate::core::Core;
use crate::events::NovaEvent;
use rmcp::service::{Peer, RoleClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

mod connection;
mod elicitation;
mod oauth;
mod sse;
mod stdio;
mod streamable_http;
mod types;

use connection::{map_rmcp_tools, McpCallError, McpConnection};
pub use elicitation::{cancel_elicitations_for_client, pending_elicitation_client_id};
use sse::connect_sse;
use stdio::connect_stdio;
use streamable_http::connect_streamable_http;

pub use types::{
    McpElicitationDecision, McpNegotiatedCapabilities, McpOAuthLoginResult, McpResourceInfo,
    McpRuntimeStatus, McpServerConfig, McpServerEntry, McpServerStatus, McpToolInfo,
};

struct RegisteredServer {
    config: McpServerConfig,
    config_revision: u64,
    enabled: bool,
    status: McpRuntimeStatus,
    tool_count: usize,
    error: Option<String>,
    connection: Option<Arc<dyn McpConnection>>,
    protocol_version: Option<String>,
    capabilities: Option<McpNegotiatedCapabilities>,
    /// 来源标注：由智能体配置页迁入/创建的 server 记录其智能体名，仅展示用。
    source_agent: Option<String>,
}

static NEXT_CONFIG_REVISION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

fn next_config_revision() -> u64 {
    NEXT_CONFIG_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

async fn shutdown_outcome_if_unused(outcome: ConnectOutcome) {
    if let Some(conn) = outcome.connection {
        conn.shutdown().await;
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PersistedServer {
    name: String,
    enabled: bool,
    config: McpServerConfig,
    /// 来源智能体名（设置页展示用）；普通全局条目为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_agent: Option<String>,
}

static MCP_RUNTIME: OnceLock<Mutex<HashMap<String, RegisteredServer>>> = OnceLock::new();
static MCP_LOADED: OnceLock<Mutex<bool>> = OnceLock::new();
const MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(60);

fn runtime() -> &'static Mutex<HashMap<String, RegisteredServer>> {
    MCP_RUNTIME.get_or_init(|| Mutex::new(HashMap::new()))
}

fn loaded_flag() -> &'static Mutex<bool> {
    MCP_LOADED.get_or_init(|| Mutex::new(false))
}

fn server_type(config: &McpServerConfig) -> String {
    match config {
        McpServerConfig::Stdio { .. } => "stdio".to_string(),
        McpServerConfig::Sse { .. } => "sse".to_string(),
        McpServerConfig::StreamableHttp { .. } => "streamable_http".to_string(),
    }
}

fn get_mcp_settings_path(core: &Core) -> Result<PathBuf, String> {
    Ok(core.config().data_dir().join("mcp_servers.json"))
}

fn load_persisted_servers(core: &Core) -> Vec<PersistedServer> {
    let Ok(path) = get_mcp_settings_path(core) else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    if let Ok(mut list) = serde_json::from_str::<Vec<PersistedServer>>(&content) {
        decrypt_persisted_server_headers(&mut list);
        return list;
    }

    // 兼容 Claude/Cline 常见的 mcpServers 对象格式。
    let value = match serde_json::from_str::<Value>(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(servers_obj) = value.get("mcpServers").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut servers = Vec::with_capacity(servers_obj.len());
    for (name, cfg_value) in servers_obj {
        let Ok(config) = serde_json::from_value::<McpServerConfig>(cfg_value.clone()) else {
            continue;
        };

        let enabled = cfg_value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        servers.push(PersistedServer {
            name: name.clone(),
            enabled,
            config,
            source_agent: None,
        });
    }

    decrypt_persisted_server_headers(&mut servers);
    servers
}

fn save_persisted_servers(core: &Core, servers: &[PersistedServer]) -> Result<(), String> {
    let path = get_mcp_settings_path(core)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut encrypted_servers = servers.to_vec();
    encrypt_persisted_server_headers(&mut encrypted_servers)?;
    let content = serde_json::to_string_pretty(&encrypted_servers).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())
}

fn encrypt_headers(headers: &mut HashMap<String, String>) -> Result<(), String> {
    for (name, value) in headers.iter_mut() {
        let trimmed = value.trim();
        if trimmed.is_empty()
            || crate::services::settings_secrets::is_encrypted_secret_value(trimmed)
        {
            continue;
        }
        *value = crate::services::settings_secrets::encrypt_secret_value(trimmed)
            .map_err(|error| format!("Failed to encrypt MCP header '{}': {}", name, error))?;
    }
    Ok(())
}

fn decrypt_headers(headers: &mut HashMap<String, String>) {
    for value in headers.values_mut() {
        let trimmed = value.trim();
        if !crate::services::settings_secrets::is_encrypted_secret_value(trimmed) {
            continue;
        }
        match crate::services::settings_secrets::decrypt_secret_value(trimmed) {
            Ok(plain) => *value = plain,
            Err(_) => value.clear(),
        }
    }
}

fn encrypt_persisted_server_headers(servers: &mut [PersistedServer]) -> Result<(), String> {
    for server in servers {
        match &mut server.config {
            McpServerConfig::Sse { headers, .. }
            | McpServerConfig::StreamableHttp { headers, .. } => encrypt_headers(headers)?,
            McpServerConfig::Stdio { .. } => {}
        }
    }
    Ok(())
}

fn decrypt_persisted_server_headers(servers: &mut [PersistedServer]) {
    for server in servers {
        match &mut server.config {
            McpServerConfig::Sse { headers, .. }
            | McpServerConfig::StreamableHttp { headers, .. } => decrypt_headers(headers),
            McpServerConfig::Stdio { .. } => {}
        }
    }
}

/// 某智能体可见的 server 名单：全局 server 按 enabled_mcp_servers 引用清单过滤。
/// 供 MCP catalog 注入和工具可见性判断使用。
pub async fn servers_visible_to_agent(
    core: &Core,
    bundle: Option<&crate::agent::capabilities::agent_bundles::AgentBundle>,
) -> Vec<(String, RegisteredServerSnapshot)> {
    ensure_runtime_loaded(core).await;
    let map = runtime().lock().await;
    let mut out = Vec::new();
    for (name, item) in map.iter() {
        let visible = match bundle {
            Some(bundle) => bundle.is_mcp_server_enabled(name),
            None => true,
        };
        if visible {
            out.push((
                name.clone(),
                RegisteredServerSnapshot {
                    enabled: item.enabled,
                    status: item.status.as_str().to_string(),
                    tool_count: item.tool_count,
                    r#type: server_type(&item.config),
                    error: item.error.clone(),
                    source_agent: item.source_agent.clone(),
                },
            ));
        }
    }
    out
}

/// 运行时 server 快照（catalog 注入用，避免长锁持有 connection）。
pub struct RegisteredServerSnapshot {
    pub enabled: bool,
    pub status: String,
    pub tool_count: usize,
    pub r#type: String,
    pub error: Option<String>,
    pub source_agent: Option<String>,
}

fn mcp_not_found(name: &str) -> crate::NovaCoreError {
    crate::NovaCoreError::NotFound(format!("MCP server '{}' not found", name))
}

fn mcp_err(server: &str, message: impl Into<String>) -> crate::NovaCoreError {
    crate::NovaCoreError::mcp(server, message)
}

async fn persist_runtime(core: &Core) -> Result<(), crate::NovaCoreError> {
    let map = runtime().lock().await;
    let mut servers = Vec::with_capacity(map.len());
    for (name, item) in map.iter() {
        servers.push(PersistedServer {
            name: name.clone(),
            enabled: item.enabled,
            config: item.config.clone(),
            source_agent: item.source_agent.clone(),
        });
    }
    drop(map);
    save_persisted_servers(core, &servers).map_err(crate::NovaCoreError::Session)
}

/// 一次性迁移：把各智能体的私有 MCP 配置（agents/<id>/mcp.json）并入全局注册表，
/// 写入 source_agent 来源标注；成功后将 mcp.json 改名为 mcp.json.migrated 备份。
/// 返回是否发生了迁移（用于触发落盘）。
fn migrate_agent_mcp_configs(
    core: &Core,
    map: &mut HashMap<String, RegisteredServer>,
) -> bool {
    let Ok(bundles) = crate::agent::capabilities::agent_bundles::list_bundles(core) else {
        return false;
    };
    let mut migrated_any = false;
    for bundle in bundles {
        let Ok(path) =
            crate::agent::capabilities::agent_bundles::agent_mcp_config_path(core, &bundle.id)
        else {
            continue;
        };
        if !path.exists() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut list) = serde_json::from_str::<Vec<PersistedServer>>(&content) else {
            // 格式无法解析：改名备份避免反复尝试，并告警。
            tracing::warn!(agent = %bundle.id, "agent mcp.json unparsable; archived as .migrated");
            let _ = std::fs::rename(&path, path.with_extension("json.migrated"));
            continue;
        };
        decrypt_persisted_server_headers(&mut list);

        for item in list {
            // 与已有条目同名时自动改名（私有条目让步）：<name>-2、<name>-3…
            let mut unique = item.name.clone();
            let mut suffix = 2;
            while map.contains_key(&unique) {
                unique = format!("{}-{}", item.name, suffix);
                suffix += 1;
            }
            if unique != item.name {
                tracing::warn!(
                    server = %item.name,
                    renamed = %unique,
                    agent = %bundle.id,
                    "agent-private MCP server renamed during migration due to name conflict"
                );
            }
            map.insert(
                unique,
                RegisteredServer {
                    config: item.config,
                    config_revision: next_config_revision(),
                    enabled: item.enabled,
                    status: McpRuntimeStatus::Disconnected,
                    tool_count: 0,
                    error: None,
                    connection: None,
                    protocol_version: None,
                    capabilities: None,
                    source_agent: Some(bundle.name.clone()),
                },
            );
            migrated_any = true;
        }

        // 迁移成功：改名备份，不直接删。
        let _ = std::fs::rename(&path, path.with_extension("json.migrated"));
    }
    migrated_any
}

struct ConnectOutcome {
    status: McpRuntimeStatus,
    tool_count: usize,
    error: Option<String>,
    connection: Option<Arc<dyn McpConnection>>,
    protocol_version: Option<String>,
    capabilities: Option<McpNegotiatedCapabilities>,
}

impl ConnectOutcome {
    fn disconnected() -> Self {
        Self {
            status: McpRuntimeStatus::Disconnected,
            tool_count: 0,
            error: None,
            connection: None,
            protocol_version: None,
            capabilities: None,
        }
    }
}

async fn open_transport(
    core: &Core,
    server_name: &str,
    config: &McpServerConfig,
) -> Result<Arc<dyn McpConnection>, String> {
    match config {
        McpServerConfig::Stdio { command, args, env } => {
            connect_stdio(core, server_name, command, args, env).await
        }
        McpServerConfig::Sse { url, headers } => connect_sse(core, server_name, url, headers).await,
        McpServerConfig::StreamableHttp { url, headers } => {
            connect_streamable_http(core, server_name, url, headers).await
        }
    }
}

async fn connect_server(core: &Core, server_name: &str, config: &McpServerConfig) -> ConnectOutcome {
    let conn = match open_transport(core, server_name, config).await {
        Ok(conn) => conn,
        Err(e) => {
            return ConnectOutcome {
                status: McpRuntimeStatus::Error,
                tool_count: 0,
                error: Some(e),
                connection: None,
                protocol_version: None,
                capabilities: None,
            };
        }
    };

    let (protocol_version, capabilities) = conn.negotiated_info();
    let timeout_hint = match config {
        McpServerConfig::Stdio { .. } => {
            "MCP tools/list timeout (30s). Server may still be downloading dependencies or stuck during startup."
        }
        McpServerConfig::Sse { .. } => "MCP tools/list timeout (30s) over SSE.",
        McpServerConfig::StreamableHttp { .. } => {
            "MCP tools/list timeout (30s). streamable_http server may be overloaded or blocked."
        }
    };

    match timeout(MCP_CONNECT_TIMEOUT, conn.list_tools()).await {
        Err(_) => {
            conn.shutdown().await;
            ConnectOutcome {
                status: McpRuntimeStatus::Error,
                tool_count: 0,
                error: Some(timeout_hint.to_string()),
                connection: None,
                protocol_version: None,
                capabilities: None,
            }
        }
        Ok(Ok(tools)) => {
            let count = tools.len();
            crate::agent::capabilities::mcp_tools::refresh_server_tools_cache(server_name, &tools);
            ConnectOutcome {
                status: McpRuntimeStatus::Connected,
                tool_count: count,
                error: None,
                connection: Some(conn),
                protocol_version,
                capabilities,
            }
        }
        Ok(Err(e)) => ConnectOutcome {
            status: McpRuntimeStatus::Connected,
            tool_count: 0,
            error: Some(e),
            connection: Some(conn),
            protocol_version,
            capabilities,
        },
    }
}

async fn apply_connect_outcome(
    server: &mut RegisteredServer,
    outcome: ConnectOutcome,
) -> Option<Arc<dyn McpConnection>> {
    let old = server.connection.take();
    server.status = outcome.status;
    server.tool_count = outcome.tool_count;
    server.error = outcome.error;
    server.connection = outcome.connection;
    server.protocol_version = outcome.protocol_version;
    server.capabilities = outcome.capabilities;
    old
}

async fn reconnect_server(core: &Core, name: &str) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(core).await;

    let (cfg, rev) = {
        let map = runtime().lock().await;
        let server = map.get(name).ok_or_else(|| mcp_not_found(name))?;
        if !server.enabled {
            return Err(mcp_err(name, format!("MCP server '{}' is disabled", name)));
        }
        (server.config.clone(), server.config_revision)
    };

    let outcome = connect_server(core, name, &cfg).await;
    let is_connected = outcome.status == McpRuntimeStatus::Connected;
    let err_msg = outcome.error.clone();

    let (old_conn, stale_outcome) = {
        let mut map = runtime().lock().await;
        match map.get_mut(name) {
            Some(server) if server.enabled && server.config_revision == rev => {
                (apply_connect_outcome(server, outcome).await, None)
            }
            Some(_) => {
                (None, Some((outcome, Some(mcp_err(name, "MCP server configuration changed during reconnect")))))
            }
            None => (None, Some((outcome, Some(mcp_not_found(name))))),
        }
    };
    if let Some(conn) = old_conn {
        conn.shutdown().await;
    }
    if let Some((unused, err)) = stale_outcome {
        shutdown_outcome_if_unused(unused).await;
        if let Some(e) = err {
            return Err(e);
        }
    }

    if is_connected {
        Ok(())
    } else {
        Err(mcp_err(
            name,
            err_msg.unwrap_or_else(|| format!("MCP server '{}' failed to reconnect", name)),
        ))
    }
}

async fn mark_server_runtime_error(server_name: &str, error: String) {
    let old_conn = {
        let mut map = runtime().lock().await;
        if let Some(server) = map.get_mut(server_name) {
            server.status = McpRuntimeStatus::Error;
            server.error = Some(error);
            server.tool_count = 0;
            server.connection.take()
        } else {
            None
        }
    };
    if let Some(conn) = old_conn {
        conn.shutdown().await;
    }
}

pub(super) async fn handle_tool_list_changed_notification(
    core: Core,
    server_name: String,
    peer: Peer<RoleClient>,
) {
    let Ok(raw_tools) = peer.list_all_tools().await else {
        return;
    };
    let tools = map_rmcp_tools(raw_tools);
    let tool_count = tools.len();
    crate::agent::capabilities::mcp_tools::refresh_server_tools_cache(&server_name, &tools);
    {
        let mut map = runtime().lock().await;
        if let Some(server) = map.get_mut(&server_name) {
            server.tool_count = tool_count;
            mark_server_connected(server);
        }
    }
    core.emit(NovaEvent::McpToolsChanged {
        server_name,
        tool_count,
    });
}

async fn ensure_runtime_loaded(core: &Core) {
    let mut loaded = loaded_flag().lock().await;
    if *loaded {
        return;
    }

    let persisted = load_persisted_servers(core);
    let migrated = {
        let mut map = runtime().lock().await;
        for item in persisted {
            map.insert(
                item.name,
                RegisteredServer {
                    config: item.config,
                    config_revision: next_config_revision(),
                    enabled: item.enabled,
                    status: McpRuntimeStatus::Disconnected,
                    tool_count: 0,
                    error: None,
                    connection: None,
                    protocol_version: None,
                    capabilities: None,
                    source_agent: item.source_agent,
                },
            );
        }

        // 存量迁移：各 agents/<id>/mcp.json 并入全局注册表（带来源标注）。
        migrate_agent_mcp_configs(core, &mut map)
    };

    // 迁移结果落盘：下次启动直接从 mcp_servers.json 读取，不会重复迁移。
    if migrated {
        persist_runtime(core).await.ok();
    }

    let names: Vec<String> = {
        let map = runtime().lock().await;
        map.iter()
            .filter_map(|(name, s)| if s.enabled { Some(name.clone()) } else { None })
            .collect()
    };

    for name in names {
        let config = {
            let map = runtime().lock().await;
            map.get(&name).map(|s| s.config.clone())
        };

        if let Some(cfg) = config {
            let outcome = connect_server(core, &name, &cfg).await;
            let mut map = runtime().lock().await;
            if let Some(server) = map.get_mut(&name) {
                let _ = apply_connect_outcome(server, outcome).await;
            }
        }
    }

    *loaded = true;
}

pub async fn add_mcp_server(
    core: Core,
    name: String,
    config: McpServerConfig,
    source_agent: Option<String>,
) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    if name.trim().is_empty() {
        return Err(crate::NovaCoreError::InvalidInput(
            "Server name cannot be empty".into(),
        ));
    }

    let outcome = connect_server(&core, &name, &config).await;

    let old_conn = {
        let mut map = runtime().lock().await;
        let old = map.remove(&name).and_then(|mut s| s.connection.take());
        map.insert(
            name,
            RegisteredServer {
                config,
                config_revision: next_config_revision(),
                enabled: true,
                status: outcome.status,
                tool_count: outcome.tool_count,
                error: outcome.error,
                connection: outcome.connection,
                protocol_version: outcome.protocol_version,
                capabilities: outcome.capabilities,
                source_agent,
            },
        );
        old
    };
    if let Some(conn) = old_conn {
        conn.shutdown().await;
    }

    persist_runtime(&core).await?;

    Ok(())
}

pub async fn get_mcp_server(
    core: Core,
    name: String,
) -> Result<McpServerEntry, crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let map = runtime().lock().await;
    let server = map.get(&name).ok_or_else(|| mcp_not_found(&name))?;

    Ok(McpServerEntry {
        name,
        enabled: server.enabled,
        config: server.config.clone(),
    })
}

pub async fn update_mcp_server(
    core: Core,
    old_name: String,
    new_name: String,
    config: McpServerConfig,
) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let old_name = old_name.trim().to_string();
    let new_name = new_name.trim().to_string();
    if old_name.is_empty() || new_name.is_empty() {
        return Err(crate::NovaCoreError::InvalidInput(
            "Server name cannot be empty".into(),
        ));
    }

    let (enabled, source_agent, rev) = {
        let map = runtime().lock().await;
        let current = map.get(&old_name).ok_or_else(|| mcp_not_found(&old_name))?;

        if old_name != new_name && map.contains_key(&new_name) {
            return Err(crate::NovaCoreError::InvalidInput(format!(
                "MCP server '{}' already exists",
                new_name
            )));
        }

        (current.enabled, current.source_agent.clone(), current.config_revision)
    };

    let outcome = if enabled {
        connect_server(&core, &new_name, &config).await
    } else {
        ConnectOutcome::disconnected()
    };

    let (old_conn, stale_outcome) = {
        let mut map = runtime().lock().await;
        if map.get(&old_name).is_none_or(|s| s.config_revision != rev) {
            (None, Some(outcome))
        } else if let Some(mut old) = map.remove(&old_name) {
            let prev = old.connection.take();
            map.insert(
                new_name,
                RegisteredServer {
                    config,
                    config_revision: next_config_revision(),
                    enabled,
                    status: outcome.status,
                    tool_count: outcome.tool_count,
                    error: outcome.error,
                    connection: outcome.connection,
                    protocol_version: outcome.protocol_version,
                    capabilities: outcome.capabilities,
                    source_agent,
                },
            );
            (prev, None)
        } else {
            (None, Some(outcome))
        }
    };
    if let Some(conn) = old_conn {
        conn.shutdown().await;
    }
    if let Some(unused) = stale_outcome {
        shutdown_outcome_if_unused(unused).await;
        return Err(mcp_not_found(&old_name));
    }

    persist_runtime(&core).await?;
    Ok(())
}

pub async fn remove_mcp_server(core: Core, name: String) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let old_conn = {
        let mut map = runtime().lock().await;
        map.remove(&name).and_then(|mut item| item.connection.take())
    };
    if let Some(conn) = old_conn {
        conn.shutdown().await;
    }
    let _ = oauth::clear_oauth_credentials(&core, &name);

    persist_runtime(&core).await?;

    Ok(())
}

pub async fn get_mcp_server_statuses(
    core: Core,
) -> Result<Vec<McpServerStatus>, crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let map = runtime().lock().await;
    let mut result = Vec::new();
    for (name, item) in map.iter() {
        let is_remote = !matches!(item.config, McpServerConfig::Stdio { .. });
        let auth_status = Some(oauth::auth_status_for_server(&core, name, is_remote));
        result.push(McpServerStatus {
            name: name.clone(),
            status: item.status.as_str().to_string(),
            enabled: item.enabled,
            r#type: server_type(&item.config),
            tool_count: item.tool_count,
            error: item.error.clone(),
            source_agent: item.source_agent.clone(),
            protocol_version: item.protocol_version.clone(),
            capabilities: item.capabilities.clone(),
            auth_status,
        });
    }
    Ok(result)
}

pub async fn reload_all_mcp_servers(core: Core) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let configs: Vec<(String, McpServerConfig, bool, u64)> = {
        let map = runtime().lock().await;
        map.iter()
            .map(|(k, v)| (k.clone(), v.config.clone(), v.enabled, v.config_revision))
            .collect()
    };

    for (name, cfg, enabled, rev) in configs {
        let outcome = if enabled {
            connect_server(&core, &name, &cfg).await
        } else {
            ConnectOutcome::disconnected()
        };
        let (old_conn, stale_outcome) = {
            let mut map = runtime().lock().await;
            match map.get_mut(&name) {
                Some(server) if server.config_revision == rev && server.enabled == enabled => {
                    (apply_connect_outcome(server, outcome).await, None)
                }
                _ => (None, Some(outcome)),
            }
        };
        if let Some(conn) = old_conn {
            conn.shutdown().await;
        }
        if let Some(unused) = stale_outcome {
            shutdown_outcome_if_unused(unused).await;
        }
    }

    persist_runtime(&core).await?;

    Ok(())
}

pub async fn set_mcp_server_enabled(
    core: Core,
    name: String,
    enabled: bool,
) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let (cfg, rev) = {
        let mut map = runtime().lock().await;
        let server = map.get_mut(&name).ok_or_else(|| mcp_not_found(&name))?;
        server.enabled = enabled;
        server.config_revision = next_config_revision();
        (server.config.clone(), server.config_revision)
    };

    let outcome = if enabled {
        connect_server(&core, &name, &cfg).await
    } else {
        ConnectOutcome::disconnected()
    };

    let (old_conn, stale_outcome) = {
        let mut map = runtime().lock().await;
        match map.get_mut(&name) {
            Some(server) if server.config_revision == rev => {
                (apply_connect_outcome(server, outcome).await, None)
            }
            _ => (None, Some(outcome)),
        }
    };
    if let Some(conn) = old_conn {
        conn.shutdown().await;
    }
    if let Some(unused) = stale_outcome {
        shutdown_outcome_if_unused(unused).await;
    }

    persist_runtime(&core).await?;
    Ok(())
}

const MCP_MAX_IN_FLIGHT_REQUESTS: usize = 32;

static MCP_IN_FLIGHT_SEMAPHORE: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();

fn mcp_semaphore() -> &'static std::sync::Arc<tokio::sync::Semaphore> {
    MCP_IN_FLIGHT_SEMAPHORE
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(MCP_MAX_IN_FLIGHT_REQUESTS)))
}

async fn acquire_mcp_permit_with(
    sem: &std::sync::Arc<tokio::sync::Semaphore>,
    wait_timeout: Duration,
    server: &str,
    operation: &str,
) -> Result<tokio::sync::OwnedSemaphorePermit, crate::NovaCoreError> {
    match timeout(wait_timeout, std::sync::Arc::clone(sem).acquire_owned()).await {
        Ok(Ok(permit)) => Ok(permit),
        Ok(Err(_)) => Err(mcp_err(
            server,
            format!("MCP concurrency semaphore closed during {}", operation),
        )),
        Err(_) => Err(mcp_err(
            server,
            format!(
                "MCP concurrency limit ({}) saturated while waiting for {}",
                MCP_MAX_IN_FLIGHT_REQUESTS, operation
            ),
        )),
    }
}

async fn acquire_mcp_permit(
    server: &str,
    operation: &str,
) -> Result<tokio::sync::OwnedSemaphorePermit, crate::NovaCoreError> {
    acquire_mcp_permit_with(mcp_semaphore(), MCP_CONNECT_TIMEOUT, server, operation).await
}

async fn ensure_server_connected(
    core: &Core,
    server_name: &str,
) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(core).await;
    let needs_reconnect = {
        let map = runtime().lock().await;
        let server = map
            .get(server_name)
            .ok_or_else(|| mcp_not_found(server_name))?;
        server.enabled && server.connection.is_none()
    };
    if needs_reconnect {
        reconnect_server(core, server_name).await?;
    }
    Ok(())
}

fn mark_server_connected(server: &mut RegisteredServer) {
    server.status = McpRuntimeStatus::Connected;
    server.error = None;
}

async fn current_connection(server_name: &str) -> Result<Arc<dyn McpConnection>, String> {
    let map = runtime().lock().await;
    let server = map
        .get(server_name)
        .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
    server
        .connection
        .clone()
        .ok_or_else(|| "Server is not connected".to_string())
}

async fn run_with_reconnect<T, F, Fut>(
    core: &Core,
    server_name: &str,
    mut op: F,
) -> Result<T, crate::NovaCoreError>
where
    F: FnMut(Arc<dyn McpConnection>) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let first_attempt = match current_connection(server_name).await {
        Ok(conn) => op(conn).await,
        Err(e) => Err(e),
    };

    match first_attempt {
        Ok(val) => Ok(val),
        Err(first_err) => {
            mark_server_runtime_error(server_name, first_err).await;
            reconnect_server(core, server_name).await?;
            let conn = current_connection(server_name)
                .await
                .map_err(|e| mcp_err(server_name, e))?;
            op(conn).await.map_err(|e| mcp_err(server_name, e))
        }
    }
}

pub async fn list_mcp_tools(
    core: Core,
    server_name: String,
) -> Result<Vec<McpToolInfo>, crate::NovaCoreError> {
    let _permit = acquire_mcp_permit(&server_name, "list_mcp_tools").await?;
    ensure_server_connected(&core, &server_name).await?;

    let tools = run_with_reconnect(&core, &server_name, |conn| async move {
        conn.list_tools().await
    })
    .await?;

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| mcp_not_found(&server_name))?;
    server.tool_count = tools.len();
    mark_server_connected(server);
    drop(map);
    crate::agent::capabilities::mcp_tools::refresh_server_tools_cache(&server_name, &tools);
    Ok(tools)
}

pub async fn list_mcp_resources(
    core: Core,
    server_name: String,
) -> Result<Vec<McpResourceInfo>, crate::NovaCoreError> {
    let _permit = acquire_mcp_permit(&server_name, "list_mcp_resources").await?;
    ensure_server_connected(&core, &server_name).await?;

    let resources = run_with_reconnect(&core, &server_name, |conn| async move {
        conn.list_resources().await
    })
    .await?;

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| mcp_not_found(&server_name))?;
    mark_server_connected(server);
    Ok(resources)
}

pub async fn read_mcp_resource(
    core: Core,
    server_name: String,
    uri: String,
) -> Result<Value, crate::NovaCoreError> {
    let _permit = acquire_mcp_permit(&server_name, "read_mcp_resource").await?;
    ensure_server_connected(&core, &server_name).await?;

    let value = run_with_reconnect(&core, &server_name, |conn| {
        let uri = uri.clone();
        async move { conn.read_resource(&uri).await }
    })
    .await?;

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| mcp_not_found(&server_name))?;
    mark_server_connected(server);
    Ok(value)
}

pub async fn call_mcp_tool(
    core: Core,
    server_name: String,
    tool_name: String,
    arguments: Value,
) -> Result<Value, crate::NovaCoreError> {
    call_mcp_tool_with_context(core, None, server_name, tool_name, arguments, None).await
}

pub(crate) async fn call_mcp_tool_with_context(
    core: Core,
    conversation_id: Option<String>,
    server_name: String,
    tool_name: String,
    arguments: Value,
    cancel_token: Option<CancellationToken>,
) -> Result<Value, crate::NovaCoreError> {
    let _permit =
        acquire_mcp_permit(&server_name, &format!("call_mcp_tool:{}/{}", server_name, tool_name))
            .await?;
    ensure_server_connected(&core, &server_name).await?;

    let effective_token = cancel_token.or_else(|| {
        conversation_id
            .as_deref()
            .map(|id| crate::agent::cancellation::get_token(Some(id)))
    });

    let scoped_server = server_name.clone();
    elicitation::scope_conversation(&scoped_server, conversation_id, async move {
        let first_attempt = match current_connection(&server_name).await {
            Ok(conn) => {
                conn.call_tool(
                    &tool_name,
                    arguments.clone(),
                    effective_token.clone(),
                    MCP_TOOL_CALL_TIMEOUT,
                )
                .await
            }
            Err(e) => Err(McpCallError::Transport(e)),
        };

        let result = match first_attempt {
            Ok(v) => v,
            Err(err) => {
                if !err.should_reconnect() {
                    return Err(mcp_err(&server_name, err.into_message()));
                }
                mark_server_runtime_error(&server_name, err.into_message()).await;
                reconnect_server(&core, &server_name).await?;
                let conn = current_connection(&server_name)
                    .await
                    .map_err(|e| mcp_err(&server_name, e))?;
                conn.call_tool(&tool_name, arguments, effective_token, MCP_TOOL_CALL_TIMEOUT)
                    .await
                    .map_err(|e| mcp_err(&server_name, e.into_message()))?
            }
        };

        let mut map = runtime().lock().await;
        let server = map
            .get_mut(&server_name)
            .ok_or_else(|| mcp_not_found(&server_name))?;
        mark_server_connected(server);
        Ok(result)
    })
    .await
}

pub async fn mcp_oauth_login(
    core: Core,
    server_name: String,
    client_id: Option<String>,
    scopes: Vec<String>,
) -> Result<McpOAuthLoginResult, crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let url = {
        let map = runtime().lock().await;
        let server = map.get(&server_name).ok_or_else(|| mcp_not_found(&server_name))?;
        match &server.config {
            McpServerConfig::Sse { url, .. } | McpServerConfig::StreamableHttp { url, .. } => {
                url.clone()
            }
            McpServerConfig::Stdio { .. } => {
                return Err(crate::NovaCoreError::InvalidInput(format!(
                    "MCP server '{}' uses stdio and does not support OAuth 2.1",
                    server_name
                )));
            }
        }
    };

    let result = oauth::perform_oauth_login(&core, &server_name, &url, client_id, scopes)
        .await
        .map_err(|e| mcp_err(&server_name, e))?;

    let _ = reconnect_server(&core, &server_name).await;
    Ok(result)
}

pub async fn mcp_oauth_logout(
    core: Core,
    server_name: String,
) -> Result<McpOAuthLoginResult, crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    {
        let map = runtime().lock().await;
        if !map.contains_key(&server_name) {
            return Err(mcp_not_found(&server_name));
        }
    }

    oauth::clear_oauth_credentials(&core, &server_name).map_err(|e| mcp_err(&server_name, e))?;
    let _ = reconnect_server(&core, &server_name).await;

    Ok(McpOAuthLoginResult {
        status: "disconnected".to_string(),
        auth_status: "none".to_string(),
        message: Some("MCP OAuth credentials cleared".to_string()),
    })
}

pub fn resolve_mcp_elicitation(
    request_id: String,
    decision: McpElicitationDecision,
    caller_client_id: Option<&str>,
) -> Result<bool, crate::NovaCoreError> {
    elicitation::submit_elicitation_decision(&request_id, decision, caller_client_id)
        .map_err(crate::NovaCoreError::InvalidInput)
}

pub async fn warmup_runtime(core: Core) -> Result<(), crate::NovaCoreError> {
    ensure_runtime_loaded(&core).await;

    let has_enabled = {
        let map = runtime().lock().await;
        map.values().any(|s| s.enabled)
    };

    if has_enabled {
        let _ = reload_all_mcp_servers(core.clone()).await;
    }

    Ok(())
}

/// 关闭全部活跃 MCP 连接与子进程（供守护进程优雅停机调用）。
pub async fn shutdown_all() {
    let conns: Vec<Arc<dyn McpConnection>> = {
        let mut map = runtime().lock().await;
        map.values_mut()
            .filter_map(|server| {
                server.status = McpRuntimeStatus::Disconnected;
                server.tool_count = 0;
                server.connection.take()
            })
            .collect()
    };
    for conn in conns {
        conn.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session_runtime::{
        complete_turn_and_take_next, request_cancel, submit_turn, SubmitTurnDecision,
    };
    use crate::CoreConfig;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::sse::{Event, KeepAlive, Sse};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::Router;
    use futures_util::StreamExt;
    use nova_protocol::agent::TurnSubmitMode;
    use serde_json::json;
    use std::sync::{Arc, Mutex as StdMutex, OnceLock};
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

    fn mcp_lib_test_lock() -> &'static AsyncMutex<()> {
        static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| AsyncMutex::new(()))
    }

    fn temp_core() -> Result<(TempDir, Core), String> {
        let dir = TempDir::new().map_err(|e| format!("tempdir failed: {e}"))?;
        let cfg = CoreConfig::new(dir.path().to_path_buf())
            .with_cache_dir(dir.path().join("cache"))
            .with_download_dir(dir.path().join("downloads"));
        Ok((dir, Core::new(cfg, 128)))
    }

    #[derive(Clone)]
    struct MockSseServerState {
        sse_tx: broadcast::Sender<String>,
        tools: Arc<StdMutex<Vec<Value>>>,
        incoming_tx: mpsc::UnboundedSender<Value>,
        elicit_responses: Arc<StdMutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>>,
    }

    async fn mock_sse_endpoint(State(state): State<MockSseServerState>) -> impl IntoResponse {
        let rx = state.sse_tx.subscribe();
        let first = futures_util::stream::once(async {
            Ok::<Event, std::convert::Infallible>(
                Event::default().event("endpoint").data("/message"),
            )
        });
        let rest = futures_util::stream::unfold(rx, |mut r| async move {
            loop {
                match r.recv().await {
                    Ok(payload) => {
                        return Some((
                            Ok::<Event, std::convert::Infallible>(
                                Event::default().event("message").data(payload),
                            ),
                            r,
                        ));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        });
        Sse::new(first.chain(rest)).keep_alive(KeepAlive::default())
    }

    async fn mock_message_endpoint(
        State(state): State<MockSseServerState>,
        axum::Json(msg): axum::Json<Value>,
    ) -> StatusCode {
        let _ = state.incoming_tx.send(msg.clone());

        // Handle responses to server-initiated requests (e.g. elicitation/create)
        if msg.get("method").is_none() {
            if let Some(id) = msg.get("id").and_then(Value::as_i64) {
                let sender = state
                    .elicit_responses
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.remove(&id));
                if let Some(tx) = sender {
                    let _ = tx.send(msg);
                }
            }
            return StatusCode::ACCEPTED;
        }

        let Some(id) = msg.get("id").cloned() else {
            // Client notification (e.g. notifications/initialized, notifications/cancelled)
            return StatusCode::ACCEPTED;
        };

        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        match method {
            "initialize" => {
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {
                            "tools": { "listChanged": true },
                            "resources": { "subscribe": true, "listChanged": true },
                            "prompts": { "listChanged": false },
                            "logging": {}
                        },
                        "serverInfo": { "name": "mock-modern-mcp", "version": "1.0.0" }
                    }
                });
                let _ = state.sse_tx.send(reply.to_string());
            }
            "tools/list" => {
                let tools = state
                    .tools
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                });
                let _ = state.sse_tx.send(reply.to_string());
            }
            "tools/call" => {
                let tool_name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let state_clone = state.clone();
                tokio::spawn(async move {
                    match tool_name.as_str() {
                        "slow_tool" => {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                        }
                        "elicit_form" => {
                            let req_id = 701_i64;
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            if let Ok(mut guard) = state_clone.elicit_responses.lock() {
                                guard.insert(req_id, tx);
                            }
                            let req = json!({
                                "jsonrpc": "2.0",
                                "id": req_id,
                                "method": "elicitation/create",
                                "params": {
                                    "mode": "form",
                                    "message": "Select target branch",
                                    "requestedSchema": {
                                        "type": "object",
                                        "properties": {
                                            "branch": { "type": "string" }
                                        },
                                        "required": ["branch"]
                                    }
                                }
                            });
                            let _ = state_clone.sse_tx.send(req.to_string());
                            let response = rx.await.unwrap_or_else(|_| json!({}));
                            let reply = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": response.get("result").cloned().unwrap_or(Value::Null).to_string()
                                    }]
                                }
                            });
                            let _ = state_clone.sse_tx.send(reply.to_string());
                        }
                        "elicit_url" => {
                            let req_id = 702_i64;
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            if let Ok(mut guard) = state_clone.elicit_responses.lock() {
                                guard.insert(req_id, tx);
                            }
                            let req = json!({
                                "jsonrpc": "2.0",
                                "id": req_id,
                                "method": "elicitation/create",
                                "params": {
                                    "mode": "url",
                                    "message": "Authorize external integration",
                                    "url": "https://example.com/authorize",
                                    "elicitationId": "elicit-url-702"
                                }
                            });
                            let _ = state_clone.sse_tx.send(req.to_string());
                            let response = rx.await.unwrap_or_else(|_| json!({}));
                            let reply = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": response.get("result").cloned().unwrap_or(Value::Null).to_string()
                                    }]
                                }
                            });
                            let _ = state_clone.sse_tx.send(reply.to_string());
                        }
                        _ => {
                            let reply = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{ "type": "text", "text": "ok" }]
                                }
                            });
                            let _ = state_clone.sse_tx.send(reply.to_string());
                        }
                    }
                });
            }
            _ => {
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" }
                });
                let _ = state.sse_tx.send(reply.to_string());
            }
        }
        StatusCode::ACCEPTED
    }

    async fn spawn_mock_modern_sse_server(
        initial_tools: Vec<Value>,
    ) -> Result<
        (
            String,
            MockSseServerState,
            mpsc::UnboundedReceiver<Value>,
            tokio::task::JoinHandle<()>,
        ),
        String,
    > {
        let (sse_tx, _) = broadcast::channel::<String>(64);
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
        let state = MockSseServerState {
            sse_tx,
            tools: Arc::new(StdMutex::new(initial_tools)),
            incoming_tx,
            elicit_responses: Arc::new(StdMutex::new(HashMap::new())),
        };
        let app = Router::new()
            .route("/sse", get(mock_sse_endpoint))
            .route("/message", post(mock_message_endpoint))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("bind failed: {e}"))?;
        let addr = listener.local_addr().map_err(|e| format!("addr failed: {e}"))?;
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((format!("http://{addr}/sse"), state, incoming_rx, handle))
    }

    fn sample_tool(name: &str) -> Value {
        json!({
            "name": name,
            "description": format!("Mock tool {name}"),
            "inputSchema": { "type": "object", "properties": {} }
        })
    }

    #[tokio::test]
    async fn mcp_concurrency_semaphore_rejects_when_saturated() -> Result<(), String> {
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
        let p1 = acquire_mcp_permit_with(&sem, Duration::from_millis(50), "srv", "op1")
            .await
            .map_err(|e| format!("first permit failed: {e}"))?;
        let p2 = acquire_mcp_permit_with(&sem, Duration::from_millis(50), "srv", "op2")
            .await
            .map_err(|e| format!("second permit failed: {e}"))?;

        let Err(err) = acquire_mcp_permit_with(&sem, Duration::from_millis(20), "srv", "op3").await
        else {
            return Err("third concurrent call must time out on semaphore".to_string());
        };
        assert!(err.to_string().contains("saturated"));

        drop(p1);
        let p3 = acquire_mcp_permit_with(&sem, Duration::from_millis(50), "srv", "op3_retry")
            .await
            .map_err(|e| format!("permit after release failed: {e}"))?;
        drop(p2);
        drop(p3);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_protocol_negotiation_and_tools_list_changed_hot_reload() -> Result<(), String> {
        let _guard = mcp_lib_test_lock().lock().await;
        let (_dir, core) = temp_core()?;
        let (sse_url, state, _incoming_rx, server_handle) =
            spawn_mock_modern_sse_server(vec![sample_tool("tool_alpha")]).await?;

        let server_name = "mock-modern-cap-reload";
        let mut events = core.events().subscribe();

        add_mcp_server(
            core.clone(),
            server_name.to_string(),
            McpServerConfig::Sse {
                url: sse_url,
                headers: HashMap::new(),
            },
            None,
        )
        .await
        .map_err(|e| format!("add_mcp_server failed: {e}"))?;

        let statuses = get_mcp_server_statuses(core.clone())
            .await
            .map_err(|e| format!("get_mcp_server_statuses failed: {e}"))?;
        let status = statuses
            .iter()
            .find(|s| s.name == server_name)
            .ok_or_else(|| "missing server status".to_string())?;
        assert_eq!(status.protocol_version.as_deref(), Some("2025-11-25"));
        let caps = status
            .capabilities
            .as_ref()
            .ok_or_else(|| "missing negotiated capabilities".to_string())?;
        assert!(caps.tools);
        assert!(caps.tools_list_changed);
        assert!(caps.resources);
        assert!(caps.prompts);
        assert!(caps.logging);
        assert_eq!(status.tool_count, 1);

        // Push notifications/tools/list_changed with an updated tool list
        if let Ok(mut guard) = state.tools.lock() {
            guard.push(sample_tool("tool_beta"));
        }
        let _ = state.sse_tx.send(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/tools/list_changed"
            })
            .to_string(),
        );

        let updated_count = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(NovaEvent::McpToolsChanged {
                        server_name: ev_srv,
                        tool_count,
                    }) if ev_srv == server_name => return Ok(tool_count),
                    Ok(_) => continue,
                    Err(e) => return Err(format!("event recv error: {e}")),
                }
            }
        })
        .await
        .map_err(|_| "timed out waiting for McpToolsChanged event".to_string())??;

        assert_eq!(updated_count, 2);
        let statuses_after = get_mcp_server_statuses(core.clone())
            .await
            .map_err(|e| format!("get_mcp_server_statuses failed: {e}"))?;
        let status_after = statuses_after
            .iter()
            .find(|s| s.name == server_name)
            .ok_or_else(|| "missing status after tools/list_changed".to_string())?;
        assert_eq!(status_after.tool_count, 2);
        assert!(
            crate::agent::capabilities::mcp_tools::find_cached_mcp_tool(
                "mcp__mock-modern-cap-reload__tool_beta"
            )
            .is_some(),
            "tool_beta must be present in mcp_tools cache"
        );

        let _ = remove_mcp_server(core, server_name.to_string()).await;
        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn mcp_cancellation_sends_notifications_cancelled_to_server() -> Result<(), String> {
        let _guard = mcp_lib_test_lock().lock().await;
        let (_dir, core) = temp_core()?;
        let (sse_url, _state, mut incoming_rx, server_handle) =
            spawn_mock_modern_sse_server(vec![sample_tool("slow_tool")]).await?;

        let server_name = "mock-modern-cancel";
        add_mcp_server(
            core.clone(),
            server_name.to_string(),
            McpServerConfig::Sse {
                url: sse_url,
                headers: HashMap::new(),
            },
            None,
        )
        .await
        .map_err(|e| format!("add_mcp_server failed: {e}"))?;

        let conv_id = "conv-mcp-cancel-test".to_string();
        let SubmitTurnDecision::StartNow { .. } = submit_turn(
            &conv_id,
            "run slow tool".to_string(),
            None,
            TurnSubmitMode::StartOrSteer,
            Some("client-cancel".to_string()),
        ) else {
            return Err("expected StartNow decision".to_string());
        };

        let call_handle = tokio::spawn({
            let core = core.clone();
            let conv_id = conv_id.clone();
            async move {
                call_mcp_tool_with_context(
                    core,
                    Some(conv_id),
                    server_name.to_string(),
                    "slow_tool".to_string(),
                    json!({}),
                    None,
                )
                .await
            }
        });

        // Wait until server actually receives the slow tools/call request
        let slow_request_id = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(msg) = incoming_rx.recv().await {
                if msg.get("method").and_then(Value::as_str) == Some("tools/call") {
                    return msg.get("id").cloned();
                }
            }
            None
        })
        .await
        .map_err(|_| "timed out waiting for slow tools/call".to_string())?
        .ok_or_else(|| "missing slow tools/call id".to_string())?;

        assert!(request_cancel(Some(&conv_id)));

        let cancelled_request_id = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(msg) = incoming_rx.recv().await {
                if msg.get("method").and_then(Value::as_str) == Some("notifications/cancelled") {
                    return msg
                        .get("params")
                        .and_then(|p| p.get("requestId"))
                        .cloned();
                }
            }
            None
        })
        .await
        .map_err(|_| "timed out waiting for notifications/cancelled".to_string())?
        .ok_or_else(|| "missing requestId in notifications/cancelled".to_string())?;

        assert_eq!(cancelled_request_id, slow_request_id);

        let call_res = tokio::time::timeout(Duration::from_secs(5), call_handle)
            .await
            .map_err(|_| "call_handle did not finish promptly after cancel".to_string())?
            .map_err(|e| format!("call_handle join error: {e}"))?;
        assert!(call_res.is_err(), "cancelled tool call must return Err");

        let _ = complete_turn_and_take_next(Some(&conv_id));
        let _ = remove_mcp_server(core, server_name.to_string()).await;
        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn mcp_elicitation_form_url_cross_client_guard_and_disconnect_fail_closed(
    ) -> Result<(), String> {
        let _guard = mcp_lib_test_lock().lock().await;
        let (_dir, core) = temp_core()?;
        let (sse_url, _state, _incoming_rx, server_handle) = spawn_mock_modern_sse_server(vec![
            sample_tool("elicit_form"),
            sample_tool("elicit_url"),
        ])
        .await?;

        let server_name = "mock-modern-elicit";
        add_mcp_server(
            core.clone(),
            server_name.to_string(),
            McpServerConfig::Sse {
                url: sse_url,
                headers: HashMap::new(),
            },
            None,
        )
        .await
        .map_err(|e| format!("add_mcp_server failed: {e}"))?;

        let conv_id = "conv-mcp-elicit-test".to_string();
        let SubmitTurnDecision::StartNow { .. } = submit_turn(
            &conv_id,
            "run elicitation".to_string(),
            None,
            TurnSubmitMode::StartOrSteer,
            Some("client-A".to_string()),
        ) else {
            return Err("expected StartNow decision".to_string());
        };

        let mut events = core.events().subscribe();

        // 1) Form elicitation + cross-client guard + accept
        let form_call = tokio::spawn({
            let core = core.clone();
            let conv_id = conv_id.clone();
            async move {
                call_mcp_tool_with_context(
                    core,
                    Some(conv_id),
                    server_name.to_string(),
                    "elicit_form".to_string(),
                    json!({}),
                    None,
                )
                .await
            }
        });

        let form_req_id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(NovaEvent::McpElicitationRequested {
                        request_id,
                        server_name: ev_srv,
                        mode,
                        requested_schema,
                        ..
                    }) if ev_srv == server_name => {
                        assert_eq!(mode, "form");
                        assert!(requested_schema.is_some());
                        return Ok(request_id);
                    }
                    Ok(_) => continue,
                    Err(e) => return Err(format!("event recv failed: {e}")),
                }
            }
        })
        .await
        .map_err(|_| "timed out waiting for form elicitation event".to_string())??;

        let wrong_client_res = resolve_mcp_elicitation(
            form_req_id.clone(),
            McpElicitationDecision {
                action: "accept".to_string(),
                content: Some(json!({ "branch": "hijacked" })),
            },
            Some("client-B"),
        );
        assert!(
            wrong_client_res.is_err(),
            "non-owning client-B must be rejected"
        );

        let accepted = resolve_mcp_elicitation(
            form_req_id,
            McpElicitationDecision {
                action: "accept".to_string(),
                content: Some(json!({ "branch": "main" })),
            },
            Some("client-A"),
        )
        .map_err(|e| format!("owning client accept failed: {e}"))?;
        assert!(accepted);

        let form_result = form_call
            .await
            .map_err(|e| format!("form call join failed: {e}"))?
            .map_err(|e| format!("form call failed: {e}"))?;
        let form_text = form_result
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(form_text.contains("\"action\":\"accept\""));
        assert!(form_text.contains("\"branch\":\"main\""));

        // 2) URL elicitation + originating client disconnect -> fail-closed cancel
        let url_call = tokio::spawn({
            let core = core.clone();
            let conv_id = conv_id.clone();
            async move {
                call_mcp_tool_with_context(
                    core,
                    Some(conv_id),
                    server_name.to_string(),
                    "elicit_url".to_string(),
                    json!({}),
                    None,
                )
                .await
            }
        });

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(NovaEvent::McpElicitationRequested {
                        server_name: ev_srv,
                        mode,
                        url,
                        ..
                    }) if ev_srv == server_name => {
                        assert_eq!(mode, "url");
                        assert_eq!(url.as_deref(), Some("https://example.com/authorize"));
                        return Ok::<(), String>(());
                    }
                    Ok(_) => continue,
                    Err(e) => return Err(format!("event recv failed: {e}")),
                }
            }
        })
        .await
        .map_err(|_| "timed out waiting for url elicitation event".to_string())??;

        let cancelled_count = cancel_elicitations_for_client("client-A");
        assert_eq!(cancelled_count, 1);

        let url_result = url_call
            .await
            .map_err(|e| format!("url call join failed: {e}"))?
            .map_err(|e| format!("url call failed: {e}"))?;
        let url_text = url_result
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(
            url_text.contains("\"action\":\"cancel\""),
            "client disconnect must fail-closed with action=cancel, got {url_text}"
        );

        let _ = complete_turn_and_take_next(Some(&conv_id));
        let _ = remove_mcp_server(core, server_name.to_string()).await;
        server_handle.abort();
        Ok(())
    }
}
