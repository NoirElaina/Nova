use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use tokio::time::timeout;

mod sse;
mod stdio;
mod streamable_http;
mod types;

use sse::connect_sse;
use stdio::{connect_stdio, StdioMcpConnection};
use streamable_http::{connect_streamable_http, StreamableHttpMcpConnection};

pub use types::{
    McpResourceInfo, McpRuntimeStatus, McpServerConfig, McpServerEntry, McpServerStatus,
    McpToolInfo,
};

enum ServerConnection {
    Stdio(StdioMcpConnection),
    StreamableHttp(StreamableHttpMcpConnection),
}

impl ServerConnection {
    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, String> {
        match self {
            Self::Stdio(conn) => conn.list_tools().await,
            Self::StreamableHttp(conn) => conn.list_tools().await,
        }
    }

    async fn list_resources(&mut self) -> Result<Vec<McpResourceInfo>, String> {
        match self {
            Self::Stdio(conn) => conn.list_resources().await,
            Self::StreamableHttp(conn) => conn.list_resources().await,
        }
    }

    async fn read_resource(&mut self, uri: &str) -> Result<Value, String> {
        match self {
            Self::Stdio(conn) => conn.read_resource(uri).await,
            Self::StreamableHttp(conn) => conn.read_resource(uri).await,
        }
    }

    async fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        match self {
            Self::Stdio(conn) => conn.call_tool(tool_name, arguments).await,
            Self::StreamableHttp(conn) => conn.call_tool(tool_name, arguments).await,
        }
    }

    async fn shutdown(&mut self) {
        match self {
            Self::Stdio(conn) => conn.shutdown().await,
            Self::StreamableHttp(conn) => conn.shutdown().await,
        }
    }
}

struct RegisteredServer {
    config: McpServerConfig,
    enabled: bool,
    status: McpRuntimeStatus,
    tool_count: usize,
    error: Option<String>,
    connection: Option<ServerConnection>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PersistedServer {
    name: String,
    enabled: bool,
    config: McpServerConfig,
}

static MCP_RUNTIME: OnceLock<Mutex<HashMap<String, RegisteredServer>>> = OnceLock::new();
static MCP_LOADED: OnceLock<Mutex<bool>> = OnceLock::new();
const MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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

fn get_mcp_settings_path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("mcp_servers.json")
}

fn load_persisted_servers(app: &AppHandle) -> Vec<PersistedServer> {
    let path = get_mcp_settings_path(app);
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
        });
    }

    decrypt_persisted_server_headers(&mut servers);
    servers
}

fn save_persisted_servers(app: &AppHandle, servers: &[PersistedServer]) -> Result<(), String> {
    let path = get_mcp_settings_path(app);
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
            || crate::command::settings_secrets::is_encrypted_secret_value(trimmed)
        {
            continue;
        }
        *value = crate::command::settings_secrets::encrypt_secret_value(trimmed)
            .map_err(|error| format!("Failed to encrypt MCP header '{}': {}", name, error))?;
    }
    Ok(())
}

fn decrypt_headers(headers: &mut HashMap<String, String>) {
    for value in headers.values_mut() {
        let trimmed = value.trim();
        if !crate::command::settings_secrets::is_encrypted_secret_value(trimmed) {
            continue;
        }
        match crate::command::settings_secrets::decrypt_secret_value(trimmed) {
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

async fn persist_runtime(app: &AppHandle) -> Result<(), String> {
    let map = runtime().lock().await;
    let mut servers = Vec::with_capacity(map.len());
    for (name, item) in map.iter() {
        servers.push(PersistedServer {
            name: name.clone(),
            enabled: item.enabled,
            config: item.config.clone(),
        });
    }
    drop(map);
    save_persisted_servers(app, &servers)
}

async fn connect_server(
    config: &McpServerConfig,
) -> (
    McpRuntimeStatus,
    usize,
    Option<String>,
    Option<ServerConnection>,
) {
    match config {
        McpServerConfig::Stdio { command, args, env } => {
            match connect_stdio(command, args, env).await {
                Ok(mut conn) => {
                    let tool_count = match timeout(MCP_CONNECT_TIMEOUT, conn.list_tools()).await {
                        Err(_) => {
                            let _ = conn.shutdown().await;
                            return (
                            McpRuntimeStatus::Error,
                            0,
                            Some("MCP tools/list timeout (30s). Server may still be downloading dependencies or stuck during startup.".to_string()),
                            None,
                        );
                        }
                        Ok(result) => match result {
                            Ok(tools) => tools.len(),
                            Err(e) => {
                                return (
                                    McpRuntimeStatus::Connected,
                                    0,
                                    Some(e),
                                    Some(ServerConnection::Stdio(conn)),
                                )
                            }
                        },
                    };
                    (
                        McpRuntimeStatus::Connected,
                        tool_count,
                        None,
                        Some(ServerConnection::Stdio(conn)),
                    )
                }
                Err(e) => (McpRuntimeStatus::Error, 0, Some(e), None),
            }
        }
        McpServerConfig::Sse { url, headers } => {
            let error = connect_sse(url, headers).err().unwrap_or_else(|| {
                "SSE MCP runtime not implemented yet. Use stdio for now.".to_string()
            });
            (McpRuntimeStatus::Error, 0, Some(error), None)
        }
        McpServerConfig::StreamableHttp { url, headers } => {
            match connect_streamable_http(url, headers).await {
                Ok(mut conn) => {
                    let tool_count = match timeout(MCP_CONNECT_TIMEOUT, conn.list_tools()).await {
                        Err(_) => {
                            let _ = conn.shutdown().await;
                            return (
                            McpRuntimeStatus::Error,
                            0,
                            Some("MCP tools/list timeout (30s). streamable_http server may be overloaded or blocked.".to_string()),
                            None,
                        );
                        }
                        Ok(result) => match result {
                            Ok(tools) => tools.len(),
                            Err(e) => {
                                return (
                                    McpRuntimeStatus::Connected,
                                    0,
                                    Some(e),
                                    Some(ServerConnection::StreamableHttp(conn)),
                                )
                            }
                        },
                    };
                    (
                        McpRuntimeStatus::Connected,
                        tool_count,
                        None,
                        Some(ServerConnection::StreamableHttp(conn)),
                    )
                }
                Err(e) => (McpRuntimeStatus::Error, 0, Some(e), None),
            }
        }
    }
}

async fn reconnect_server(app: &AppHandle, name: &str) -> Result<(), String> {
    ensure_runtime_loaded(app).await;

    let cfg = {
        let map = runtime().lock().await;
        let server = map
            .get(name)
            .ok_or_else(|| format!("MCP server '{}' not found", name))?;
        if !server.enabled {
            return Err(format!("MCP server '{}' is disabled", name));
        }
        server.config.clone()
    };

    let (status, tool_count, error, connection) = connect_server(&cfg).await;
    let mut map = runtime().lock().await;
    let server = map
        .get_mut(name)
        .ok_or_else(|| format!("MCP server '{}' not found", name))?;

    if let Some(conn) = server.connection.as_mut() {
        conn.shutdown().await;
    }

    server.status = status.clone();
    server.tool_count = tool_count;
    server.error = error.clone();
    server.connection = connection;

    if status == McpRuntimeStatus::Connected {
        Ok(())
    } else {
        Err(error.unwrap_or_else(|| format!("MCP server '{}' failed to reconnect", name)))
    }
}

async fn mark_server_runtime_error(server_name: &str, error: String) {
    let mut map = runtime().lock().await;
    if let Some(server) = map.get_mut(server_name) {
        server.status = McpRuntimeStatus::Error;
        server.error = Some(error);
        server.tool_count = 0;
        server.connection = None;
    }
}

async fn ensure_runtime_loaded(app: &AppHandle) {
    let mut loaded = loaded_flag().lock().await;
    if *loaded {
        return;
    }

    let persisted = load_persisted_servers(app);
    {
        let mut map = runtime().lock().await;
        for item in persisted {
            map.insert(
                item.name,
                RegisteredServer {
                    config: item.config,
                    enabled: item.enabled,
                    status: McpRuntimeStatus::Disconnected,
                    tool_count: 0,
                    error: None,
                    connection: None,
                },
            );
        }
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
            let (status, tool_count, error, connection) = connect_server(&cfg).await;
            let mut map = runtime().lock().await;
            if let Some(server) = map.get_mut(&name) {
                server.status = status;
                server.tool_count = tool_count;
                server.error = error;
                server.connection = connection;
            }
        }
    }

    *loaded = true;
}

pub async fn add_mcp_server(
    app: AppHandle,
    name: String,
    config: McpServerConfig,
) -> Result<(), String> {
    ensure_runtime_loaded(&app).await;

    if name.trim().is_empty() {
        return Err("Server name cannot be empty".into());
    }

    let (status, connected_tools, error, connection) = connect_server(&config).await;

    let mut map = runtime().lock().await;
    if let Some(mut old) = map.remove(&name) {
        if let Some(conn) = old.connection.as_mut() {
            conn.shutdown().await;
        }
    }

    map.insert(
        name,
        RegisteredServer {
            config,
            enabled: true,
            status,
            tool_count: connected_tools,
            error,
            connection,
        },
    );
    drop(map);

    persist_runtime(&app).await?;

    Ok(())
}

pub async fn get_mcp_server(app: AppHandle, name: String) -> Result<McpServerEntry, String> {
    ensure_runtime_loaded(&app).await;

    let map = runtime().lock().await;
    let server = map
        .get(&name)
        .ok_or_else(|| format!("MCP server '{}' not found", name))?;

    Ok(McpServerEntry {
        name,
        enabled: server.enabled,
        config: server.config.clone(),
    })
}

pub async fn update_mcp_server(
    app: AppHandle,
    old_name: String,
    new_name: String,
    config: McpServerConfig,
) -> Result<(), String> {
    ensure_runtime_loaded(&app).await;

    let old_name = old_name.trim().to_string();
    let new_name = new_name.trim().to_string();
    if old_name.is_empty() || new_name.is_empty() {
        return Err("Server name cannot be empty".into());
    }

    let enabled = {
        let map = runtime().lock().await;
        let current = map
            .get(&old_name)
            .ok_or_else(|| format!("MCP server '{}' not found", old_name))?;

        if old_name != new_name && map.contains_key(&new_name) {
            return Err(format!("MCP server '{}' already exists", new_name));
        }

        current.enabled
    };

    let (status, connected_tools, error, connection) = if enabled {
        connect_server(&config).await
    } else {
        (McpRuntimeStatus::Disconnected, 0, None, None)
    };

    let mut map = runtime().lock().await;
    let mut old = map
        .remove(&old_name)
        .ok_or_else(|| format!("MCP server '{}' not found", old_name))?;

    if let Some(conn) = old.connection.as_mut() {
        conn.shutdown().await;
    }

    map.insert(
        new_name,
        RegisteredServer {
            config,
            enabled,
            status,
            tool_count: connected_tools,
            error,
            connection,
        },
    );
    drop(map);

    persist_runtime(&app).await?;
    Ok(())
}

pub async fn remove_mcp_server(app: AppHandle, name: String) -> Result<(), String> {
    ensure_runtime_loaded(&app).await;

    let mut map = runtime().lock().await;
    if let Some(mut item) = map.remove(&name) {
        if let Some(conn) = item.connection.as_mut() {
            conn.shutdown().await;
        }
    }
    drop(map);

    persist_runtime(&app).await?;

    Ok(())
}

pub async fn get_mcp_server_statuses(app: AppHandle) -> Result<Vec<McpServerStatus>, String> {
    ensure_runtime_loaded(&app).await;

    let map = runtime().lock().await;
    let mut result = Vec::new();
    for (name, item) in map.iter() {
        result.push(McpServerStatus {
            name: name.clone(),
            status: item.status.as_str().to_string(),
            enabled: item.enabled,
            r#type: server_type(&item.config),
            tool_count: item.tool_count,
            error: item.error.clone(),
        });
    }
    Ok(result)
}

pub async fn reload_all_mcp_servers(app: AppHandle) -> Result<(), String> {
    ensure_runtime_loaded(&app).await;

    let configs: Vec<(String, McpServerConfig, bool)> = {
        let map = runtime().lock().await;
        map.iter()
            .map(|(k, v)| (k.clone(), v.config.clone(), v.enabled))
            .collect()
    };

    for (name, cfg, enabled) in configs {
        if !enabled {
            let mut map = runtime().lock().await;
            if let Some(server) = map.get_mut(&name) {
                server.status = McpRuntimeStatus::Disconnected;
                server.tool_count = 0;
                server.error = None;
                if let Some(conn) = server.connection.as_mut() {
                    conn.shutdown().await;
                }
                server.connection = None;
            }
            continue;
        }

        let (status, tool_count, error, connection) = connect_server(&cfg).await;
        let mut map = runtime().lock().await;
        if let Some(server) = map.get_mut(&name) {
            if let Some(conn) = server.connection.as_mut() {
                conn.shutdown().await;
            }
            server.status = status;
            server.tool_count = tool_count;
            server.error = error;
            server.connection = connection;
        }
    }

    persist_runtime(&app).await?;

    Ok(())
}

pub async fn set_mcp_server_enabled(
    app: AppHandle,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    ensure_runtime_loaded(&app).await;

    let cfg = {
        let map = runtime().lock().await;
        let server = map
            .get(&name)
            .ok_or_else(|| format!("MCP server '{}' not found", name))?;
        server.config.clone()
    };

    if enabled {
        let (status, tool_count, error, connection) = connect_server(&cfg).await;
        let mut map = runtime().lock().await;
        if let Some(server) = map.get_mut(&name) {
            if let Some(conn) = server.connection.as_mut() {
                conn.shutdown().await;
            }
            server.enabled = true;
            server.status = status;
            server.tool_count = tool_count;
            server.error = error;
            server.connection = connection;
        }
    } else {
        let mut map = runtime().lock().await;
        if let Some(server) = map.get_mut(&name) {
            if let Some(conn) = server.connection.as_mut() {
                conn.shutdown().await;
            }
            server.enabled = false;
            server.status = McpRuntimeStatus::Disconnected;
            server.tool_count = 0;
            server.error = None;
            server.connection = None;
        }
    }

    persist_runtime(&app).await?;
    Ok(())
}

pub async fn list_mcp_tools(
    app: AppHandle,
    server_name: String,
) -> Result<Vec<McpToolInfo>, String> {
    ensure_runtime_loaded(&app).await;

    let needs_reconnect = {
        let map = runtime().lock().await;
        let server = map
            .get(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
        server.enabled && server.connection.is_none()
    };

    if needs_reconnect {
        reconnect_server(&app, &server_name).await?;
    }

    let first_attempt = {
        let mut map = runtime().lock().await;
        let server = map
            .get_mut(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

        match server.connection.as_mut() {
            Some(conn) => conn.list_tools().await,
            None => Err("Server is not connected".into()),
        }
    };

    let tools = match first_attempt {
        Ok(v) => v,
        Err(e) => {
            mark_server_runtime_error(&server_name, e.clone()).await;
            reconnect_server(&app, &server_name).await?;

            let mut map = runtime().lock().await;
            let server = map
                .get_mut(&server_name)
                .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

            match server.connection.as_mut() {
                Some(conn) => conn.list_tools().await?,
                None => return Err("Server is not connected".into()),
            }
        }
    };

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
    server.tool_count = tools.len();
    server.status = McpRuntimeStatus::Connected;
    server.error = None;
    Ok(tools)
}

pub async fn list_mcp_resources(
    app: AppHandle,
    server_name: String,
) -> Result<Vec<McpResourceInfo>, String> {
    ensure_runtime_loaded(&app).await;

    let needs_reconnect = {
        let map = runtime().lock().await;
        let server = map
            .get(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
        server.enabled && server.connection.is_none()
    };

    if needs_reconnect {
        reconnect_server(&app, &server_name).await?;
    }

    let first_attempt = {
        let mut map = runtime().lock().await;
        let server = map
            .get_mut(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

        match server.connection.as_mut() {
            Some(conn) => conn.list_resources().await,
            None => Err("Server is not connected".into()),
        }
    };

    let resources = match first_attempt {
        Ok(v) => v,
        Err(e) => {
            mark_server_runtime_error(&server_name, e.clone()).await;
            reconnect_server(&app, &server_name).await?;

            let mut map = runtime().lock().await;
            let server = map
                .get_mut(&server_name)
                .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

            match server.connection.as_mut() {
                Some(conn) => conn.list_resources().await?,
                None => return Err("Server is not connected".into()),
            }
        }
    };

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
    server.status = McpRuntimeStatus::Connected;
    server.error = None;
    Ok(resources)
}

pub async fn read_mcp_resource(
    app: AppHandle,
    server_name: String,
    uri: String,
) -> Result<Value, String> {
    ensure_runtime_loaded(&app).await;

    let needs_reconnect = {
        let map = runtime().lock().await;
        let server = map
            .get(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
        server.enabled && server.connection.is_none()
    };

    if needs_reconnect {
        reconnect_server(&app, &server_name).await?;
    }

    let first_attempt = {
        let mut map = runtime().lock().await;
        let server = map
            .get_mut(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

        match server.connection.as_mut() {
            Some(conn) => conn.read_resource(&uri).await,
            None => Err("Server is not connected".into()),
        }
    };

    let value = match first_attempt {
        Ok(v) => v,
        Err(e) => {
            mark_server_runtime_error(&server_name, e.clone()).await;
            reconnect_server(&app, &server_name).await?;

            let mut map = runtime().lock().await;
            let server = map
                .get_mut(&server_name)
                .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

            match server.connection.as_mut() {
                Some(conn) => conn.read_resource(&uri).await?,
                None => return Err("Server is not connected".into()),
            }
        }
    };

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
    server.status = McpRuntimeStatus::Connected;
    server.error = None;
    Ok(value)
}

pub async fn call_mcp_tool(
    app: AppHandle,
    server_name: String,
    tool_name: String,
    arguments: Value,
) -> Result<Value, String> {
    ensure_runtime_loaded(&app).await;

    let needs_reconnect = {
        let map = runtime().lock().await;
        let server = map
            .get(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
        server.enabled && server.connection.is_none()
    };

    if needs_reconnect {
        reconnect_server(&app, &server_name).await?;
    }

    let first_attempt = {
        let mut map = runtime().lock().await;
        let server = map
            .get_mut(&server_name)
            .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

        match server.connection.as_mut() {
            Some(conn) => conn.call_tool(&tool_name, arguments.clone()).await,
            None => Err("Server is not connected".into()),
        }
    };

    let result = match first_attempt {
        Ok(v) => v,
        Err(e) => {
            mark_server_runtime_error(&server_name, e.clone()).await;
            reconnect_server(&app, &server_name).await?;

            let mut map = runtime().lock().await;
            let server = map
                .get_mut(&server_name)
                .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;

            match server.connection.as_mut() {
                Some(conn) => conn.call_tool(&tool_name, arguments).await?,
                None => return Err("Server is not connected".into()),
            }
        }
    };

    let mut map = runtime().lock().await;
    let server = map
        .get_mut(&server_name)
        .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
    server.status = McpRuntimeStatus::Connected;
    server.error = None;
    Ok(result)
}

pub async fn warmup_runtime(app: AppHandle) -> Result<(), String> {
    ensure_runtime_loaded(&app).await;

    let has_enabled = {
        let map = runtime().lock().await;
        map.values().any(|s| s.enabled)
    };

    if has_enabled {
        let _ = reload_all_mcp_servers(app.clone()).await;
    }

    Ok(())
}
