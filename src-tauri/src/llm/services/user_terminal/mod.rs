use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub const USER_TERMINAL_EVENT: &str = "user-terminal-output";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTerminalInfo {
    pub conversation_id: Option<String>,
    pub session_id: String,
    pub cwd: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTerminalEvent {
    pub conversation_id: Option<String>,
    pub session_id: String,
    pub kind: String,
    pub data: Option<String>,
    pub exit_code: Option<u32>,
    pub error: Option<String>,
}

struct UserTerminalSession {
    conversation_id: Option<String>,
    session_id: String,
    cwd: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

static USER_TERMINALS: OnceLock<Mutex<HashMap<String, Arc<UserTerminalSession>>>> = OnceLock::new();

fn sessions() -> &'static Mutex<HashMap<String, Arc<UserTerminalSession>>> {
    USER_TERMINALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__default__")
        .to_string()
}

fn normalize_conversation_id(conversation_id: Option<&str>) -> Option<String> {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn pty_size(rows: Option<u16>, cols: Option<u16>) -> PtySize {
    PtySize {
        rows: rows.unwrap_or(24).clamp(2, 500),
        cols: cols.unwrap_or(80).clamp(10, 500),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn shell_command(root: &Path) -> CommandBuilder {
    #[cfg(target_os = "windows")]
    {
        let preferred = Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe");
        let program = if preferred.exists() {
            preferred.as_os_str()
        } else {
            std::ffi::OsStr::new("pwsh.exe")
        };
        let mut command = CommandBuilder::new(program);
        command.arg("-NoLogo");
        command.cwd(std::ffi::OsString::from(
            crate::command::workspace::display_path_string(root),
        ));
        command.env("TERM", "xterm-256color");
        command
    }

    #[cfg(not(target_os = "windows"))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
        let mut command = CommandBuilder::new(shell);
        command.cwd(root.as_os_str());
        command.env("TERM", "xterm-256color");
        command
    }
}

fn is_alive(session: &UserTerminalSession) -> bool {
    let Ok(mut child) = session.child.lock() else {
        return false;
    };
    matches!(child.try_wait(), Ok(None))
}

fn info_from_session(session: &UserTerminalSession) -> UserTerminalInfo {
    UserTerminalInfo {
        conversation_id: session.conversation_id.clone(),
        session_id: session.session_id.clone(),
        cwd: session.cwd.clone(),
    }
}

fn emit_event(app: &AppHandle, payload: UserTerminalEvent) {
    let _ = app.emit(USER_TERMINAL_EVENT, payload);
}

fn spawn_reader(
    app: AppHandle,
    conversation_id: Option<String>,
    session_id: String,
    mut reader: Box<dyn Read + Send>,
) {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let data = String::from_utf8_lossy(&buffer[..read]).to_string();
                    emit_event(
                        &app,
                        UserTerminalEvent {
                            conversation_id: conversation_id.clone(),
                            session_id: session_id.clone(),
                            kind: "output".to_string(),
                            data: Some(data),
                            exit_code: None,
                            error: None,
                        },
                    );
                }
                Err(error) => {
                    emit_event(
                        &app,
                        UserTerminalEvent {
                            conversation_id: conversation_id.clone(),
                            session_id: session_id.clone(),
                            kind: "error".to_string(),
                            data: None,
                            exit_code: None,
                            error: Some(error.to_string()),
                        },
                    );
                    break;
                }
            }
        }

        emit_event(
            &app,
            UserTerminalEvent {
                conversation_id,
                session_id,
                kind: "exit".to_string(),
                data: None,
                exit_code: None,
                error: None,
            },
        );
    });
}

pub fn start_session(
    app: AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<UserTerminalInfo, String> {
    let key = session_key(conversation_id);
    let size = pty_size(rows, cols);

    let existing = {
        sessions()
            .lock()
            .map_err(|_| "用户终端会话锁已损坏".to_string())?
            .get(&key)
            .cloned()
    };

    if let Some(existing) = existing {
        if is_alive(&existing) {
            existing
                .master
                .lock()
                .map_err(|_| "用户终端 PTY 锁已损坏".to_string())?
                .resize(size)
                .map_err(|error| format!("调整终端尺寸失败: {}", error))?;
            return Ok(info_from_session(&existing));
        }

        sessions()
            .lock()
            .map_err(|_| "用户终端会话锁已损坏".to_string())?
            .remove(&key);
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|error| format!("创建 PTY 失败: {}", error))?;
    let command = shell_command(root);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("启动终端进程失败: {}", error))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("创建终端输出流失败: {}", error))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("创建终端输入流失败: {}", error))?;

    let conversation_id = normalize_conversation_id(conversation_id);
    let session = Arc::new(UserTerminalSession {
        conversation_id: conversation_id.clone(),
        session_id: Uuid::new_v4().to_string(),
        cwd: crate::command::workspace::display_path_string(root),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
    });

    sessions()
        .lock()
        .map_err(|_| "用户终端会话锁已损坏".to_string())?
        .insert(key, session.clone());

    spawn_reader(app, conversation_id, session.session_id.clone(), reader);

    Ok(info_from_session(&session))
}

pub fn write_session(conversation_id: Option<&str>, data: String) -> Result<(), String> {
    let key = session_key(conversation_id);
    let session = sessions()
        .lock()
        .map_err(|_| "用户终端会话锁已损坏".to_string())?
        .get(&key)
        .cloned()
        .ok_or_else(|| "用户终端尚未启动".to_string())?;

    let mut writer = session
        .writer
        .lock()
        .map_err(|_| "用户终端输入流锁已损坏".to_string())?;
    writer
        .write_all(data.as_bytes())
        .map_err(|error| format!("写入终端失败: {}", error))?;
    writer
        .flush()
        .map_err(|error| format!("刷新终端输入失败: {}", error))
}

pub fn resize_session(
    conversation_id: Option<&str>,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<(), String> {
    let key = session_key(conversation_id);
    let session = sessions()
        .lock()
        .map_err(|_| "用户终端会话锁已损坏".to_string())?
        .get(&key)
        .cloned()
        .ok_or_else(|| "用户终端尚未启动".to_string())?;

    let result = session
        .master
        .lock()
        .map_err(|_| "用户终端 PTY 锁已损坏".to_string())?
        .resize(pty_size(rows, cols))
        .map_err(|error| format!("调整终端尺寸失败: {}", error));
    result
}

pub fn stop_session(conversation_id: Option<&str>) -> Result<(), String> {
    let key = session_key(conversation_id);
    let session = sessions()
        .lock()
        .map_err(|_| "用户终端会话锁已损坏".to_string())?
        .remove(&key);

    if let Some(session) = session {
        if let Ok(mut child) = session.child.lock() {
            let _ = child.kill();
        }
    }

    Ok(())
}

pub fn close_all_sessions() {
    let sessions = sessions()
        .lock()
        .ok()
        .map(|mut sessions| {
            sessions
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for session in sessions {
        if let Ok(mut child) = session.child.lock() {
            let _ = child.kill();
        }
    }
}
