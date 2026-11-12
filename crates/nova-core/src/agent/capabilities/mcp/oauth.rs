use crate::core::Core;
use crate::services::settings_secrets::{decrypt_secret_value, encrypt_secret_value};
use async_trait::async_trait;
use nova_protocol::mcp::McpOAuthLoginResult;
use rmcp::transport::auth::{
    AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession,
    CredentialStore, StoredCredentials,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const OAUTH_STORE_FILE: &str = "mcp_oauth_credentials.json";
const OAUTH_LOGIN_TIMEOUT: Duration = Duration::from_secs(120);

fn store_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredServerOAuthRecord {
    server_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credentials: Option<StoredCredentials>,
    #[serde(default)]
    auth_required: bool,
}

fn credentials_path(core: &Core) -> PathBuf {
    core.config().data_dir().join(OAUTH_STORE_FILE)
}

fn load_all_records(path: &Path) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str::<HashMap<String, String>>(&raw).unwrap_or_default()
}

fn save_all_records(path: &Path, map: &HashMap<String, String>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(path, raw).map_err(|e| e.to_string())
}

fn read_server_record(path: &Path, server_name: &str) -> Result<Option<StoredServerOAuthRecord>, String> {
    let _guard = store_lock()
        .lock()
        .map_err(|_| "OAuth store lock poisoned".to_string())?;
    let encrypted_map = load_all_records(path);
    let Some(cipher) = encrypted_map.get(server_name) else {
        return Ok(None);
    };
    let plaintext = decrypt_secret_value(cipher)?;
    let record = serde_json::from_str::<StoredServerOAuthRecord>(&plaintext)
        .map_err(|e| format!("Failed to parse OAuth credential record: {e}"))?;
    Ok(Some(record))
}

fn write_server_record(
    path: &Path,
    server_name: &str,
    record: Option<&StoredServerOAuthRecord>,
) -> Result<(), String> {
    let _guard = store_lock()
        .lock()
        .map_err(|_| "OAuth store lock poisoned".to_string())?;
    let mut encrypted_map = load_all_records(path);
    match record {
        Some(rec) => {
            let plaintext = serde_json::to_string(rec).map_err(|e| e.to_string())?;
            let cipher = encrypt_secret_value(&plaintext)?;
            encrypted_map.insert(server_name.to_string(), cipher);
        }
        None => {
            encrypted_map.remove(server_name);
        }
    }
    save_all_records(path, &encrypted_map)
}

#[derive(Clone)]
pub(super) struct EncryptedMcpCredentialStore {
    store_path: PathBuf,
    server_name: String,
    server_url: String,
    client_id: Option<String>,
    scopes: Vec<String>,
}

impl EncryptedMcpCredentialStore {
    pub(super) fn new(
        core: &Core,
        server_name: impl Into<String>,
        server_url: impl Into<String>,
        client_id: Option<String>,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            store_path: credentials_path(core),
            server_name: server_name.into(),
            server_url: server_url.into(),
            client_id,
            scopes,
        }
    }
}

#[async_trait]
impl CredentialStore for EncryptedMcpCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let record = read_server_record(&self.store_path, &self.server_name)
            .map_err(AuthError::CredentialStoreError)?;
        match record {
            Some(rec) if rec.server_url == self.server_url => Ok(rec.credentials),
            _ => Ok(None),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let mut existing = read_server_record(&self.store_path, &self.server_name)
            .map_err(AuthError::CredentialStoreError)?
            .unwrap_or_default();
        existing.server_url = self.server_url.clone();
        if self.client_id.is_some() {
            existing.client_id = self.client_id.clone();
        } else if existing.client_id.is_none() {
            existing.client_id = Some(credentials.client_id.clone());
        }
        if !self.scopes.is_empty() {
            existing.scopes = self.scopes.clone();
        }
        existing.auth_required = false;
        existing.credentials = Some(credentials);
        write_server_record(&self.store_path, &self.server_name, Some(&existing))
            .map_err(AuthError::CredentialStoreError)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        write_server_record(&self.store_path, &self.server_name, None)
            .map_err(AuthError::CredentialStoreError)
    }
}

/// 查询指定远端 MCP 服务当前持久化的 OAuth 状态。
pub(super) fn auth_status_for_server(core: &Core, server_name: &str, is_remote: bool) -> String {
    if !is_remote {
        return "none".to_string();
    }
    let path = credentials_path(core);
    match read_server_record(&path, server_name) {
        Ok(Some(rec)) if rec.credentials.is_some() => "authenticated".to_string(),
        Ok(Some(rec)) if rec.auth_required => "unauthenticated".to_string(),
        Ok(_) => "none".to_string(),
        Err(_) => "error".to_string(),
    }
}

/// 标记远端 MCP 服务触发了 401/OAuth 鉴权需求，供前端设置页呈现“登录”操作。
pub(super) fn mark_server_auth_required(core: &Core, server_name: &str, server_url: &str) {
    let path = credentials_path(core);
    let mut rec = read_server_record(&path, server_name)
        .ok()
        .flatten()
        .unwrap_or_default();
    rec.server_url = server_url.to_string();
    rec.auth_required = true;
    rec.credentials = None;
    let _ = write_server_record(&path, server_name, Some(&rec));
}

/// 当持久化凭证存在并可载入时，构造带自动 Refresh Token 的 `AuthorizationManager`。
pub(super) async fn load_authorized_manager(
    core: &Core,
    server_name: &str,
    server_url: &str,
) -> Result<Option<AuthorizationManager>, String> {
    let path = credentials_path(core);
    let Some(rec) = read_server_record(&path, server_name)? else {
        return Ok(None);
    };
    if rec.credentials.is_none() || rec.server_url != server_url {
        return Ok(None);
    }

    let mut manager = AuthorizationManager::new_allowing_private_addresses(server_url)
        .await
        .map_err(|e| format!("Failed to initialize OAuth manager: {e}"))?;
    manager.set_credential_store(EncryptedMcpCredentialStore::new(
        core,
        server_name,
        server_url,
        rec.client_id,
        rec.scopes,
    ));
    let initialized = manager
        .initialize_from_store()
        .await
        .map_err(|e| format!("Failed to load stored OAuth credentials: {e}"))?;
    if initialized {
        Ok(Some(manager))
    } else {
        Ok(None)
    }
}

/// 当服务持有有效 OAuth 凭证时，获取最新 Bearer Access Token（若临近过期会自动刷新）。
pub(super) async fn resolve_bearer_token_if_configured(
    core: &Core,
    server_name: &str,
    server_url: &str,
) -> Result<Option<String>, String> {
    let Some(manager) = load_authorized_manager(core, server_name, server_url).await? else {
        return Ok(None);
    };
    let token = manager
        .get_access_token()
        .await
        .map_err(|e| format!("MCP OAuth token refresh failed (please re-authenticate): {e}"))?;
    Ok(Some(token))
}

/// 用 `rmcp::transport::auth::AuthClient` 包装 `reqwest::Client`。
pub(super) fn wrap_reqwest_with_auth(
    client: reqwest::Client,
    manager: AuthorizationManager,
) -> AuthClient<reqwest::Client> {
    AuthClient::new(client, manager)
}

async fn probe_www_authenticate(client: &reqwest::Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    resp.headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

async fn accept_loopback_oauth_callback(listener: TcpListener) -> Result<(String, String), String> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| format!("Loopback OAuth listener accept failed: {e}"))?;

    let mut buf = vec![0u8; 8192];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Failed to read OAuth callback request: {e}"))?;
    let req_text = String::from_utf8_lossy(&buf[..n]);
    let request_line = req_text.lines().next().unwrap_or_default();
    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "Malformed HTTP callback request".to_string())?;
    let parsed = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|e| format!("Invalid callback URL: {e}"))?;

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => error_description = Some(v.into_owned()),
            _ => {}
        }
    }

    if let Some(err) = error {
        let msg = error_description.unwrap_or_else(|| err.clone());
        let body = format!(
            "<html><body><h3>MCP OAuth 授权失败</h3><p>{}</p></body></html>",
            msg.replace('<', "&lt;")
        );
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return Err(format!("OAuth authorization denied: {msg}"));
    }

    let body = "<html><body><h3>Nova MCP 授权成功</h3><p>您可以关闭此页面并返回 Nova。</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;

    let code = code.ok_or_else(|| "OAuth callback missing 'code' parameter".to_string())?;
    let state = state.ok_or_else(|| "OAuth callback missing 'state' parameter".to_string())?;
    Ok((code, state))
}

/// 启动 OAuth 2.1 Authorization Code + PKCE 完整授权流程（供 API 与集成测试共用）。
///
/// `open_browser_or_hook` 接收生成的授权 URL，实际场景默认调用系统浏览器打开，
/// 集成测试可传入自定义回调以自动化模拟授权重定向。
pub(crate) async fn perform_oauth_login_with<F, Fut>(
    core: &Core,
    server_name: &str,
    server_url: &str,
    client_id: Option<String>,
    scopes: Vec<String>,
    open_auth_url: F,
) -> Result<McpOAuthLoginResult, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind local OAuth callback port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to inspect local OAuth callback address: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let mut manager = AuthorizationManager::new_allowing_private_addresses(server_url)
        .await
        .map_err(|e| format!("Failed to create OAuth authorization manager: {e}"))?;
    manager.set_credential_store(EncryptedMcpCredentialStore::new(
        core,
        server_name,
        server_url,
        client_id.clone(),
        scopes.clone(),
    ));

    let probe_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to construct OAuth HTTP client: {e}"))?;
    let challenge = probe_www_authenticate(&probe_client, server_url).await;

    let resolution = manager
        .resolve_metadata_from_challenge(challenge.as_deref())
        .await
        .map_err(|e| format!("MCP OAuth metadata discovery failed: {e}"))?;
    manager.set_metadata(resolution.metadata);

    let mut auth_req = AuthorizationRequest::new(redirect_uri)
        .with_client_name("Nova MCP Client")
        .with_application_type("native");
    if let Some(cid) = client_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        auth_req = auth_req.with_preregistered_client(cid);
    }
    if !scopes.is_empty() {
        auth_req = auth_req.with_scopes(scopes);
    } else {
        auth_req = auth_req.with_initial_scope_selection();
    }
    if let Some(ch) = challenge {
        auth_req = auth_req.with_challenge(ch);
    }

    let session = AuthorizationSession::new(manager, auth_req)
        .await
        .map_err(|(_, e)| format!("MCP OAuth client registration/session setup failed: {e}"))?;
    let auth_url = session.get_authorization_url().to_string();

    let callback_handle = tokio::spawn(async move {
        tokio::time::timeout(OAUTH_LOGIN_TIMEOUT, accept_loopback_oauth_callback(listener)).await
    });

    open_auth_url(auth_url).await?;

    let (code, state) = match callback_handle.await {
        Ok(Ok(Ok(pair))) => pair,
        Ok(Ok(Err(err))) => return Err(err),
        Ok(Err(_)) => return Err("OAuth authorization timed out after 120s".to_string()),
        Err(join_err) => return Err(format!("OAuth callback task failed: {join_err}")),
    };

    session
        .handle_callback(&code, &state)
        .await
        .map_err(|e| format!("OAuth code-to-token exchange failed: {e}"))?;

    Ok(McpOAuthLoginResult {
        status: "connected".to_string(),
        auth_status: "authenticated".to_string(),
        message: Some("MCP OAuth 2.1 authorization succeeded".to_string()),
    })
}

/// 使用系统默认浏览器执行 OAuth 2.1 授权码 + PKCE 登录。
pub(super) async fn perform_oauth_login(
    core: &Core,
    server_name: &str,
    server_url: &str,
    client_id: Option<String>,
    scopes: Vec<String>,
) -> Result<McpOAuthLoginResult, String> {
    perform_oauth_login_with(
        core,
        server_name,
        server_url,
        client_id,
        scopes,
        |auth_url| async move {
            tokio::task::spawn_blocking(move || open::that(&auth_url))
                .await
                .map_err(|e| format!("Browser launch task failed: {e}"))?
                .map_err(|e| format!("Failed to open system browser for OAuth login: {e}"))?;
            Ok(())
        },
    )
    .await
}

/// 清除指定 MCP 服务的持久化 OAuth 凭证。
pub(super) fn clear_oauth_credentials(core: &Core, server_name: &str) -> Result<(), String> {
    let path = credentials_path(core);
    write_server_record(&path, server_name, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreConfig;
    use axum::extract::{Query, State};
    use axum::http::{HeaderValue, StatusCode};
    use axum::response::{IntoResponse, Redirect};
    use axum::routing::{get, post};
    use axum::{Form, Json, Router};
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex as StdMutex};
    use tempfile::TempDir;

    #[derive(Clone)]
    struct MockOAuthState {
        base_url: String,
        challenges: Arc<StdMutex<HashMap<String, String>>>,
        token_calls: Arc<StdMutex<u32>>,
    }

    async fn mock_mcp_unauthorized(State(state): State<MockOAuthState>) -> impl IntoResponse {
        let header_val = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\", scope=\"mcp:tools\"",
            state.base_url
        );
        let mut resp = (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        if let Ok(val) = HeaderValue::from_str(&header_val) {
            resp.headers_mut()
                .insert(axum::http::header::WWW_AUTHENTICATE, val);
        }
        resp
    }

    async fn mock_protected_resource(State(state): State<MockOAuthState>) -> Json<Value> {
        Json(json!({
            "resource": format!("{}/mcp", state.base_url),
            "authorization_servers": [state.base_url],
            "scopes_supported": ["mcp:tools"]
        }))
    }

    async fn mock_auth_server_metadata(State(state): State<MockOAuthState>) -> Json<Value> {
        Json(json!({
            "issuer": state.base_url,
            "authorization_endpoint": format!("{}/authorize", state.base_url),
            "token_endpoint": format!("{}/token", state.base_url),
            "registration_endpoint": format!("{}/register", state.base_url),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "scopes_supported": ["mcp:tools"],
            "token_endpoint_auth_methods_supported": ["none"]
        }))
    }

    async fn mock_register(Json(payload): Json<Value>) -> impl IntoResponse {
        let redirect_uris = payload
            .get("redirect_uris")
            .cloned()
            .unwrap_or_else(|| json!([]));
        (
            StatusCode::CREATED,
            Json(json!({
                "client_id": "mock-dcr-client-id",
                "client_id_issued_at": 1700000000u64,
                "redirect_uris": redirect_uris,
                "token_endpoint_auth_method": "none"
            })),
        )
    }

    #[derive(Deserialize)]
    struct AuthorizeQuery {
        redirect_uri: String,
        state: String,
        code_challenge: Option<String>,
        code_challenge_method: Option<String>,
    }

    async fn mock_authorize(
        State(state): State<MockOAuthState>,
        Query(q): Query<AuthorizeQuery>,
    ) -> impl IntoResponse {
        if q.code_challenge_method.as_deref() != Some("S256") {
            return (StatusCode::BAD_REQUEST, "expected S256 PKCE").into_response();
        }
        let Some(challenge) = q.code_challenge else {
            return (StatusCode::BAD_REQUEST, "missing code_challenge").into_response();
        };
        if let Ok(mut guard) = state.challenges.lock() {
            guard.insert(q.state.clone(), challenge);
        }
        let Ok(mut target) = reqwest::Url::parse(&q.redirect_uri) else {
            return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
        };
        target
            .query_pairs_mut()
            .append_pair("code", "mock-auth-code")
            .append_pair("state", &q.state);
        Redirect::temporary(target.as_str()).into_response()
    }

    #[derive(Deserialize)]
    struct TokenForm {
        grant_type: String,
        code: Option<String>,
        code_verifier: Option<String>,
    }

    async fn mock_token(
        State(state): State<MockOAuthState>,
        Form(form): Form<TokenForm>,
    ) -> impl IntoResponse {
        if form.grant_type != "authorization_code" || form.code.as_deref() != Some("mock-auth-code") {
            return (StatusCode::BAD_REQUEST, "invalid authorization code").into_response();
        }
        let Some(verifier) = form.code_verifier else {
            return (StatusCode::BAD_REQUEST, "missing code_verifier").into_response();
        };
        let digest = Sha256::digest(verifier.as_bytes());
        let computed_challenge = URL_SAFE_NO_PAD.encode(digest);
        let challenge_matched = state
            .challenges
            .lock()
            .map(|guard| guard.values().any(|expected| expected == &computed_challenge))
            .unwrap_or(false);
        if !challenge_matched {
            return (StatusCode::BAD_REQUEST, "PKCE verifier mismatch").into_response();
        }
        if let Ok(mut calls) = state.token_calls.lock() {
            *calls += 1;
        }
        Json(json!({
            "access_token": "secret-access-token-xyz",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "mock-refresh-abc",
            "scope": "mcp:tools"
        }))
        .into_response()
    }

    #[tokio::test]
    async fn oauth_pkce_loopback_encrypted_store_roundtrip() -> Result<(), String> {
        crate::services::settings_secrets::init_ephemeral_test_master_key();
        let dir = TempDir::new().map_err(|e| format!("tempdir failed: {e}"))?;
        let cfg = CoreConfig::new(dir.path().to_path_buf())
            .with_cache_dir(dir.path().join("cache"))
            .with_download_dir(dir.path().join("downloads"));
        let core = Core::new(cfg, 64);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("bind failed: {e}"))?;
        let addr = listener.local_addr().map_err(|e| format!("addr failed: {e}"))?;
        let base_url = format!("http://{addr}");
        let mcp_url = format!("{base_url}/mcp");

        let challenges = Arc::new(StdMutex::new(HashMap::new()));
        let token_calls = Arc::new(StdMutex::new(0u32));
        let oauth_state = MockOAuthState {
            base_url: base_url.clone(),
            challenges,
            token_calls: token_calls.clone(),
        };

        let app = Router::new()
            .route("/mcp", get(mock_mcp_unauthorized))
            .route(
                "/.well-known/oauth-protected-resource",
                get(mock_protected_resource),
            )
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(mock_protected_resource),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(mock_auth_server_metadata),
            )
            .route("/register", post(mock_register))
            .route("/authorize", get(mock_authorize))
            .route("/token", post(mock_token))
            .with_state(oauth_state);

        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let server_name = "mock-oauth-mcp";
        assert_eq!(auth_status_for_server(&core, server_name, true), "none");

        let login = perform_oauth_login_with(
            &core,
            server_name,
            &mcp_url,
            None,
            vec!["mcp:tools".to_string()],
            |auth_url| async move {
                let client = reqwest::Client::new();
                let resp = client
                    .get(&auth_url)
                    .send()
                    .await
                    .map_err(|e| format!("mock browser navigation failed: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("unexpected callback page status {}", resp.status()));
                }
                Ok(())
            },
        )
        .await?;

        assert_eq!(login.status, "connected");
        assert_eq!(login.auth_status, "authenticated");
        assert_eq!(auth_status_for_server(&core, server_name, true), "authenticated");
        assert_eq!(
            token_calls.lock().map(|g| *g).unwrap_or(0),
            1,
            "expected exactly one authorization_code token exchange"
        );

        let raw_store = std::fs::read_to_string(credentials_path(&core))
            .map_err(|e| format!("read encrypted store failed: {e}"))?;
        assert!(
            !raw_store.contains("secret-access-token-xyz"),
            "OAuth access token must not be stored in plaintext"
        );
        assert!(
            !raw_store.contains("mock-refresh-abc"),
            "OAuth refresh token must not be stored in plaintext"
        );

        let bearer = resolve_bearer_token_if_configured(&core, server_name, &mcp_url).await?;
        assert_eq!(bearer.as_deref(), Some("secret-access-token-xyz"));

        clear_oauth_credentials(&core, server_name)?;
        assert_eq!(auth_status_for_server(&core, server_name, true), "none");
        let after_logout = resolve_bearer_token_if_configured(&core, server_name, &mcp_url).await?;
        assert_eq!(after_logout, None);

        server_task.abort();
        Ok(())
    }
}
