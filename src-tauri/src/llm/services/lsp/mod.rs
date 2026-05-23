mod types;

pub use types::{
    LspDiagnostic, LspDiagnosticsResponse, LspHoverResponse, LspLocation, LspRequestResponse,
    LspServerStatus, LspStatusResponse, LspSymbolsResponse,
};

use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};

const REQUEST_TIMEOUT_MS: u64 = 15_000;
const DIAGNOSTIC_WAIT_MS: u64 = 900;

#[derive(Debug, Clone)]
struct CommandSpec {
    command: &'static str,
    args: &'static [&'static str],
}

#[derive(Debug, Clone)]
struct LanguageServerSpec {
    language_id: &'static str,
    display_name: &'static str,
    extensions: &'static [&'static str],
    commands: &'static [CommandSpec],
}

#[derive(Debug, Clone)]
struct ResolvedCommand {
    command: String,
    args: Vec<String>,
}

struct LspSession {
    spec: LanguageServerSpec,
    root: PathBuf,
    command: ResolvedCommand,
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    diagnostics: Arc<Mutex<HashMap<String, Vec<LspDiagnostic>>>>,
    open_documents: Mutex<HashMap<String, i32>>,
    next_id: AtomicI64,
    last_error: Mutex<Option<String>>,
}

type SessionMap = HashMap<String, Arc<LspSession>>;

static SESSIONS: OnceLock<Mutex<SessionMap>> = OnceLock::new();

fn sessions() -> &'static Mutex<SessionMap> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn language_server_specs() -> Vec<LanguageServerSpec> {
    vec![
        LanguageServerSpec {
            language_id: "rust",
            display_name: "Rust Analyzer",
            extensions: &["rs"],
            commands: &[CommandSpec {
                command: "rust-analyzer",
                args: &[],
            }],
        },
        LanguageServerSpec {
            language_id: "typescript",
            display_name: "TypeScript",
            extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
            commands: &[CommandSpec {
                command: "typescript-language-server",
                args: &["--stdio"],
            }],
        },
        LanguageServerSpec {
            language_id: "vue",
            display_name: "Vue",
            extensions: &["vue"],
            commands: &[
                CommandSpec {
                    command: "vue-language-server",
                    args: &["--stdio"],
                },
                CommandSpec {
                    command: "vls",
                    args: &["--stdio"],
                },
            ],
        },
        LanguageServerSpec {
            language_id: "python",
            display_name: "Pyright",
            extensions: &["py", "pyi"],
            commands: &[CommandSpec {
                command: "pyright-langserver",
                args: &["--stdio"],
            }],
        },
        LanguageServerSpec {
            language_id: "go",
            display_name: "gopls",
            extensions: &["go"],
            commands: &[CommandSpec {
                command: "gopls",
                args: &[],
            }],
        },
        LanguageServerSpec {
            language_id: "clangd",
            display_name: "clangd",
            extensions: &["c", "cc", "cpp", "cxx", "h", "hpp", "hh"],
            commands: &[CommandSpec {
                command: "clangd",
                args: &[],
            }],
        },
    ]
}

fn spec_for_path(path: &Path) -> Option<LanguageServerSpec> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    language_server_specs()
        .into_iter()
        .find(|spec| spec.extensions.iter().any(|item| *item == extension))
}

fn session_key(root: &Path, language_id: &str) -> String {
    format!("{}::{}", root.display(), language_id)
}

fn executable_names(command: &str) -> Vec<String> {
    let path = Path::new(command);
    if path.extension().is_some() {
        return vec![command.to_string()];
    }

    #[cfg(target_os = "windows")]
    {
        let mut names = vec![command.to_string()];
        let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string());
        for extension in extensions.split(';') {
            let trimmed = extension.trim();
            if !trimmed.is_empty() {
                names.push(format!("{}{}", command, trimmed.to_ascii_lowercase()));
                names.push(format!("{}{}", command, trimmed.to_ascii_uppercase()));
            }
        }
        names
    }

    #[cfg(not(target_os = "windows"))]
    {
        vec![command.to_string()]
    }
}

fn find_in_workspace_bin(root: &Path, command: &str) -> Option<PathBuf> {
    let bin_dir = root.join("node_modules").join(".bin");
    executable_names(command)
        .into_iter()
        .map(|name| bin_dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn find_on_path(command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.is_absolute() || command.contains('\\') || command.contains('/') {
        return command_path.is_file().then(|| command_path.to_path_buf());
    }

    let path_value = env::var_os("PATH")?;
    for dir in env::split_paths(&path_value) {
        for name in executable_names(command) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_command(root: &Path, spec: &LanguageServerSpec) -> Option<ResolvedCommand> {
    for command in spec.commands {
        if let Some(path) =
            find_in_workspace_bin(root, command.command).or_else(|| find_on_path(command.command))
        {
            return Some(ResolvedCommand {
                command: path.display().to_string(),
                args: command.args.iter().map(|item| item.to_string()).collect(),
            });
        }
    }
    None
}

fn build_command(resolved: &ResolvedCommand, root: &Path) -> Command {
    #[cfg(target_os = "windows")]
    let is_batch = Path::new(&resolved.command)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "cmd" | "bat"))
        .unwrap_or(false);

    #[cfg(target_os = "windows")]
    let mut command = if is_batch {
        let mut command = Command::new("cmd.exe");
        command.arg("/C").arg(&resolved.command);
        command
    } else {
        Command::new(&resolved.command)
    };

    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(&resolved.command);

    command
        .args(&resolved.args)
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        command.creation_flags(0x08000000);
    }

    command
}

fn percent_encode_uri_path(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~' | '/' | ':') {
            encoded.push(ch);
        } else {
            encoded.push_str(&format!("%{:02X}", byte));
        }
    }
    encoded
}

fn percent_decode_uri_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                output.push(hex);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn path_to_uri(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    #[cfg(target_os = "windows")]
    let normalized = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{}", normalized)
    };

    format!("file://{}", percent_encode_uri_path(&normalized))
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let without_scheme = uri.strip_prefix("file://")?;
    let decoded = percent_decode_uri_path(without_scheme);
    #[cfg(target_os = "windows")]
    {
        let path = if decoded.len() > 3 && decoded.as_bytes().get(2) == Some(&b':') {
            decoded.trim_start_matches('/').to_string()
        } else {
            decoded
        };
        Some(PathBuf::from(path))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(PathBuf::from(decoded))
    }
}

fn relative_to_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|value| {
            value
                .components()
                .filter_map(|component| match component {
                    std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| path.display().to_string())
}

async fn read_message(stdout: &mut BufReader<ChildStdout>) -> Result<Option<Value>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = stdout
            .read_line(&mut line)
            .await
            .map_err(|error| format!("读取 LSP 消息头失败: {}", error))?;
        if read == 0 {
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }

    let Some(length) = content_length else {
        return Err("LSP 消息缺少 Content-Length".to_string());
    };

    let mut body = vec![0_u8; length];
    stdout
        .read_exact(&mut body)
        .await
        .map_err(|error| format!("读取 LSP 消息体失败: {}", error))?;
    serde_json::from_slice(&body).map(Some).map_err(|error| {
        format!(
            "解析 LSP 消息失败: {}: {}",
            error,
            String::from_utf8_lossy(&body)
        )
    })
}

fn diagnostic_from_value(root: &Path, uri: &str, value: &Value) -> Option<LspDiagnostic> {
    let path = uri_to_path(uri)?;
    let range = value.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end")?;
    let line = start.get("line")?.as_u64()? + 1;
    let character = start.get("character")?.as_u64()? + 1;
    let end_line = end.get("line").and_then(Value::as_u64).unwrap_or(line - 1) + 1;
    let end_character = end
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(character - 1)
        + 1;
    Some(LspDiagnostic {
        uri: uri.to_string(),
        path: path.display().to_string(),
        relative_path: relative_to_root(root, &path),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        severity: value.get("severity").and_then(Value::as_u64),
        source: value
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string),
        code: value.get("code").map(|code| {
            code.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| code.to_string())
        }),
        line,
        character,
        end_line,
        end_character,
    })
}

async fn read_loop(session: Arc<LspSession>, stdout: ChildStdout) {
    let mut stdout = BufReader::new(stdout);
    loop {
        let message = match read_message(&mut stdout).await {
            Ok(Some(message)) => message,
            Ok(None) => {
                *session.last_error.lock().await = Some("语言服务器已退出".to_string());
                break;
            }
            Err(error) => {
                *session.last_error.lock().await = Some(error);
                break;
            }
        };

        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);

        if method.is_none() {
            if let Some(id_value) = id.and_then(|value| value.as_i64()) {
                if let Some(sender) = session.pending.lock().await.remove(&id_value) {
                    let payload = message
                        .get("result")
                        .cloned()
                        .or_else(|| message.get("error").cloned())
                        .unwrap_or(Value::Null);
                    let _ = sender.send(payload);
                }
            }
            continue;
        }

        match method.unwrap_or_default() {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = message.get("params") {
                    if let Some(uri) = params.get("uri").and_then(Value::as_str) {
                        let diagnostics = params
                            .get("diagnostics")
                            .and_then(Value::as_array)
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| {
                                        diagnostic_from_value(&session.root, uri, item)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        session
                            .diagnostics
                            .lock()
                            .await
                            .insert(uri.to_string(), diagnostics);
                    }
                }
            }
            _ => {}
        }

        if let Some(id_value) = id {
            let _ = session
                .send_raw(json!({
                    "jsonrpc": "2.0",
                    "id": id_value,
                    "result": Value::Null
                }))
                .await;
        }
    }
}

impl LspSession {
    async fn send_raw(&self, message: Value) -> Result<(), String> {
        let body = serde_json::to_string(&message)
            .map_err(|error| format!("序列化 LSP 消息失败: {}", error))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.as_bytes().len());
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|error| format!("写入 LSP 消息头失败: {}", error))?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|error| format!("写入 LSP 消息体失败: {}", error))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("刷新 LSP 输入失败: {}", error))
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        self.send_raw(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;

        match tokio::time::timeout(Duration::from_millis(REQUEST_TIMEOUT_MS), receiver).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(format!("LSP 请求 {} 未返回结果", method)),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("LSP 请求 {} 超时", method))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send_raw(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn is_running(&self) -> bool {
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    async fn ensure_open_document(&self, path: &Path) -> Result<String, String> {
        let uri = path_to_uri(path);
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| format!("读取文件失败: {}", error))?;
        let mut documents = self.open_documents.lock().await;
        if let Some(version) = documents.get_mut(&uri) {
            *version += 1;
            self.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "version": *version
                    },
                    "contentChanges": [
                        { "text": text }
                    ]
                }),
            )
            .await?;
        } else {
            documents.insert(uri.clone(), 1);
            self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": self.spec.language_id,
                        "version": 1,
                        "text": text
                    }
                }),
            )
            .await?;
        }
        Ok(uri)
    }

    async fn diagnostic_count(&self) -> usize {
        self.diagnostics.lock().await.values().map(Vec::len).sum()
    }
}

async fn start_session(
    root: PathBuf,
    spec: LanguageServerSpec,
    command: ResolvedCommand,
) -> Result<Arc<LspSession>, String> {
    let mut process = build_command(&command, &root);
    let mut child = process
        .spawn()
        .map_err(|error| format!("启动语言服务器失败: {}", error))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "语言服务器 stdin 不可用".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "语言服务器 stdout 不可用".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "语言服务器 stderr 不可用".to_string())?;
    let session = Arc::new(LspSession {
        spec,
        root: root.clone(),
        command,
        child: Mutex::new(child),
        stdin: Arc::new(Mutex::new(stdin)),
        pending: Arc::new(Mutex::new(HashMap::new())),
        diagnostics: Arc::new(Mutex::new(HashMap::new())),
        open_documents: Mutex::new(HashMap::new()),
        next_id: AtomicI64::new(1),
        last_error: Mutex::new(None),
    });

    tokio::spawn(read_loop(session.clone(), stdout));
    tokio::spawn(async move {
        let mut stderr = stderr;
        let mut buffer = [0_u8; 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    session
        .request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": path_to_uri(&root),
                "workspaceFolders": [
                    {
                        "uri": path_to_uri(&root),
                        "name": root.file_name().and_then(|item| item.to_str()).unwrap_or("workspace")
                    }
                ],
                "capabilities": {
                    "textDocument": {
                        "synchronization": {
                            "dynamicRegistration": false,
                            "didSave": true
                        },
                        "publishDiagnostics": {
                            "relatedInformation": true
                        },
                        "definition": {
                            "linkSupport": true
                        },
                        "references": {},
                        "documentSymbol": {
                            "hierarchicalDocumentSymbolSupport": true
                        },
                        "hover": {
                            "contentFormat": ["markdown", "plaintext"]
                        }
                    },
                    "workspace": {
                        "symbol": {},
                        "workspaceFolders": true
                    }
                }
            }),
        )
        .await?;

    session.notify("initialized", json!({})).await?;
    Ok(session)
}

async fn get_session_for_file(root: &Path, path: &Path) -> Result<Arc<LspSession>, String> {
    let spec = spec_for_path(path).ok_or_else(|| {
        format!(
            "当前文件类型没有内置 LSP 配置: {}",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        )
    })?;
    let key = session_key(root, spec.language_id);

    if let Some(existing) = sessions().lock().await.get(&key).cloned() {
        if existing.is_running().await {
            return Ok(existing);
        }
    }

    let command = resolve_command(root, &spec).ok_or_else(|| {
        format!(
            "未找到 {} 语言服务器，请安装命令: {}",
            spec.display_name,
            spec.commands
                .iter()
                .map(|command| command.command)
                .collect::<Vec<_>>()
                .join(" / ")
        )
    })?;

    let session = start_session(root.to_path_buf(), spec.clone(), command).await?;
    sessions().lock().await.insert(key, session.clone());
    Ok(session)
}

fn lsp_position(line: u64, character: u64) -> Value {
    json!({
        "line": line.saturating_sub(1),
        "character": character.saturating_sub(1)
    })
}

fn collect_locations(root: &Path, value: &Value, output: &mut Vec<LspLocation>) {
    if let Some(items) = value.as_array() {
        for item in items {
            collect_locations(root, item, output);
        }
        return;
    }

    let Some(object) = value.as_object() else {
        return;
    };

    let uri = object
        .get("uri")
        .or_else(|| object.get("targetUri"))
        .and_then(Value::as_str);
    let range = object
        .get("range")
        .or_else(|| object.get("targetSelectionRange"))
        .or_else(|| object.get("targetRange"));

    let (Some(uri), Some(range)) = (uri, range) else {
        return;
    };
    let Some(path) = uri_to_path(uri) else {
        return;
    };

    let start = range.get("start").unwrap_or(&Value::Null);
    let end = range.get("end").unwrap_or(&Value::Null);
    output.push(LspLocation {
        uri: uri.to_string(),
        path: path.display().to_string(),
        relative_path: relative_to_root(root, &path),
        line: start.get("line").and_then(Value::as_u64).unwrap_or(0) + 1,
        character: start.get("character").and_then(Value::as_u64).unwrap_or(0) + 1,
        end_line: end.get("line").and_then(Value::as_u64).unwrap_or(0) + 1,
        end_character: end.get("character").and_then(Value::as_u64).unwrap_or(0) + 1,
    });
}

fn resolve_workspace_file(root: &Path, path: String) -> Result<(PathBuf, String), String> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err("文件路径不能为空".to_string());
    }

    let candidate = PathBuf::from(raw);
    let target = if candidate.is_absolute() {
        candidate
    } else {
        root.join(raw)
    };
    let canonical = target
        .canonicalize()
        .map_err(|error| format!("无法解析文件路径: {}", error))?;
    if !canonical.starts_with(root) {
        return Err("拒绝访问工作区之外的文件".to_string());
    }
    if !canonical.is_file() {
        return Err("目标路径不是文件".to_string());
    }
    let relative = relative_to_root(root, &canonical);
    Ok((canonical, relative))
}

pub async fn status(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<LspStatusResponse, String> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    let mut servers = Vec::new();
    for spec in language_server_specs() {
        let key = session_key(&root, spec.language_id);
        let existing = sessions().lock().await.get(&key).cloned();
        let running = if let Some(session) = existing.as_ref() {
            session.is_running().await
        } else {
            false
        };
        let diagnostic_count = if let Some(session) = existing.as_ref() {
            session.diagnostic_count().await
        } else {
            0
        };
        let error = if let Some(session) = existing.as_ref() {
            session.last_error.lock().await.clone()
        } else {
            None
        };
        let command = existing
            .as_ref()
            .map(|session| session.command.command.clone())
            .or_else(|| resolve_command(&root, &spec).map(|command| command.command));

        servers.push(LspServerStatus {
            language_id: spec.language_id.to_string(),
            display_name: spec.display_name.to_string(),
            command,
            available: resolve_command(&root, &spec).is_some(),
            running,
            diagnostic_count,
            error,
        });
    }

    Ok(LspStatusResponse {
        workspace_root: root.display().to_string(),
        servers,
    })
}

pub async fn diagnostics(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: Option<String>,
) -> Result<LspDiagnosticsResponse, String> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    if let Some(path) = path {
        let (target, _) = resolve_workspace_file(&root, path)?;
        let Some(spec) = spec_for_path(&target) else {
            return Ok(LspDiagnosticsResponse {
                workspace_root: root.display().to_string(),
                server: None,
                diagnostics: Vec::new(),
            });
        };
        if resolve_command(&root, &spec).is_none() {
            return Ok(LspDiagnosticsResponse {
                workspace_root: root.display().to_string(),
                server: Some(spec.display_name.to_string()),
                diagnostics: Vec::new(),
            });
        }
        let session = get_session_for_file(&root, &target).await?;
        let uri = session.ensure_open_document(&target).await?;
        tokio::time::sleep(Duration::from_millis(DIAGNOSTIC_WAIT_MS)).await;
        let diagnostics = session
            .diagnostics
            .lock()
            .await
            .get(&uri)
            .cloned()
            .unwrap_or_default();
        return Ok(LspDiagnosticsResponse {
            workspace_root: root.display().to_string(),
            server: Some(session.spec.display_name.to_string()),
            diagnostics,
        });
    }

    let mut diagnostics = Vec::new();
    for session in sessions().lock().await.values() {
        if session.root == root {
            for values in session.diagnostics.lock().await.values() {
                diagnostics.extend(values.clone());
            }
        }
    }

    Ok(LspDiagnosticsResponse {
        workspace_root: root.display().to_string(),
        server: None,
        diagnostics,
    })
}

pub async fn definition(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: String,
    line: u64,
    character: u64,
) -> Result<LspRequestResponse, String> {
    text_document_location_request(
        app,
        conversation_id,
        path,
        line,
        character,
        "textDocument/definition",
        json!({}),
    )
    .await
}

pub async fn references(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: String,
    line: u64,
    character: u64,
    include_declaration: bool,
) -> Result<LspRequestResponse, String> {
    text_document_location_request(
        app,
        conversation_id,
        path,
        line,
        character,
        "textDocument/references",
        json!({
            "context": {
                "includeDeclaration": include_declaration
            }
        }),
    )
    .await
}

async fn text_document_location_request(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: String,
    line: u64,
    character: u64,
    method: &str,
    extra: Value,
) -> Result<LspRequestResponse, String> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    let (target, _) = resolve_workspace_file(&root, path)?;
    let session = get_session_for_file(&root, &target).await?;
    let uri = session.ensure_open_document(&target).await?;
    let mut params = serde_json::Map::new();
    params.insert("textDocument".to_string(), json!({ "uri": uri }));
    params.insert("position".to_string(), lsp_position(line, character));
    if let Some(extra_object) = extra.as_object() {
        for (key, value) in extra_object {
            params.insert(key.clone(), value.clone());
        }
    }

    let result = session.request(method, Value::Object(params)).await?;
    let mut locations = Vec::new();
    collect_locations(&root, &result, &mut locations);
    Ok(LspRequestResponse {
        workspace_root: root.display().to_string(),
        server: session.spec.display_name.to_string(),
        result,
        locations,
    })
}

pub async fn symbols(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: Option<String>,
    query: Option<String>,
) -> Result<LspSymbolsResponse, String> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    if let Some(path) = path {
        let (target, _) = resolve_workspace_file(&root, path)?;
        let session = get_session_for_file(&root, &target).await?;
        let uri = session.ensure_open_document(&target).await?;
        let result = session
            .request(
                "textDocument/documentSymbol",
                json!({ "textDocument": { "uri": uri } }),
            )
            .await?;
        return Ok(LspSymbolsResponse {
            workspace_root: root.display().to_string(),
            server: session.spec.display_name.to_string(),
            result,
        });
    }

    let query = query.unwrap_or_default();
    let mut last_error = None;
    for spec in language_server_specs() {
        let Some(command) = resolve_command(&root, &spec) else {
            continue;
        };
        let key = session_key(&root, spec.language_id);
        let session = if let Some(existing) = sessions().lock().await.get(&key).cloned() {
            existing
        } else {
            let session = start_session(root.clone(), spec.clone(), command).await?;
            sessions().lock().await.insert(key, session.clone());
            session
        };
        match session
            .request("workspace/symbol", json!({ "query": query }))
            .await
        {
            Ok(result) => {
                return Ok(LspSymbolsResponse {
                    workspace_root: root.display().to_string(),
                    server: session.spec.display_name.to_string(),
                    result,
                })
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| "没有可用的语言服务器可执行 workspace symbols".to_string()))
}

pub async fn hover(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: String,
    line: u64,
    character: u64,
) -> Result<LspHoverResponse, String> {
    let root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    let (target, _) = resolve_workspace_file(&root, path)?;
    let session = get_session_for_file(&root, &target).await?;
    let uri = session.ensure_open_document(&target).await?;
    let result = session
        .request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": uri },
                "position": lsp_position(line, character)
            }),
        )
        .await?;
    Ok(LspHoverResponse {
        workspace_root: root.display().to_string(),
        server: session.spec.display_name.to_string(),
        result,
    })
}
