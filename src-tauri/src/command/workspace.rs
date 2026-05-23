use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    name: String,
    path: String,
    relative_path: String,
    kind: String,
    extension: Option<String>,
    size: Option<u64>,
    modified: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDirectoryListing {
    root: String,
    path: String,
    relative_path: String,
    entries: Vec<WorkspaceEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileContent {
    path: String,
    relative_path: String,
    content: String,
    size: u64,
}

const MAX_TEXT_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize, Default)]
struct ConversationWorkspaceStore {
    #[serde(default)]
    roots: HashMap<String, String>,
}

static WORKSPACE_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn workspace_store_lock() -> &'static Mutex<()> {
    WORKSPACE_STORE_LOCK.get_or_init(|| Mutex::new(()))
}

fn normalize_conversation_id(conversation_id: Option<&str>) -> Option<String> {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn workspace_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("conversation_workspaces.json"))
        .map_err(|error| format!("无法读取应用数据目录: {}", error))
}

fn read_workspace_store(app: &AppHandle) -> Result<ConversationWorkspaceStore, String> {
    let path = workspace_store_path(app)?;
    if !path.exists() {
        return Ok(ConversationWorkspaceStore::default());
    }

    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("读取工作区配置失败: {}", error))?;
    if text.trim().is_empty() {
        return Ok(ConversationWorkspaceStore::default());
    }

    serde_json::from_str(&text).map_err(|error| format!("解析工作区配置失败: {}", error))
}

fn write_workspace_store(
    app: &AppHandle,
    store: &ConversationWorkspaceStore,
) -> Result<(), String> {
    let path = workspace_store_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建工作区配置目录失败: {}", error))?;
    }

    let text = serde_json::to_string_pretty(store)
        .map_err(|error| format!("序列化工作区配置失败: {}", error))?;
    std::fs::write(&path, text).map_err(|error| format!("保存工作区配置失败: {}", error))
}

pub fn default_workspace_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map(|path| path.join("workspace"))
        .map_err(|error| format!("无法读取默认工作区目录: {}", error))?;

    std::fs::create_dir_all(&root).map_err(|error| format!("创建默认工作区失败: {}", error))?;
    root.canonicalize()
        .map_err(|error| format!("无法解析默认工作区目录: {}", error))
}

pub fn workspace_root_for_conversation(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<PathBuf, String> {
    let Some(conversation_id) = normalize_conversation_id(conversation_id) else {
        return default_workspace_root(app);
    };

    let _guard = workspace_store_lock()
        .lock()
        .map_err(|_| "工作区配置锁已损坏".to_string())?;
    let store = read_workspace_store(app)?;
    let Some(root) = store.roots.get(&conversation_id) else {
        return default_workspace_root(app);
    };

    let canonical = PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("无法解析会话工作区目录: {}", error))?;
    if !canonical.is_dir() {
        return Err("会话工作区必须是目录".to_string());
    }

    Ok(canonical)
}

pub fn workspace_root_string_for_conversation(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<String, String> {
    workspace_root_for_conversation(app, conversation_id).map(|path| path.display().to_string())
}

pub fn remove_conversation_workspace(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let Some(conversation_id) = normalize_conversation_id(Some(conversation_id)) else {
        return Ok(());
    };

    let _guard = workspace_store_lock()
        .lock()
        .map_err(|_| "工作区配置锁已损坏".to_string())?;
    let mut store = read_workspace_store(app)?;
    store.roots.remove(&conversation_id);
    write_workspace_store(app, &store)
}

pub fn clear_conversation_workspaces(app: &AppHandle) -> Result<(), String> {
    let _guard = workspace_store_lock()
        .lock()
        .map_err(|_| "工作区配置锁已损坏".to_string())?;
    write_workspace_store(app, &ConversationWorkspaceStore::default())
}

fn set_workspace_root_for_conversation(
    app: &AppHandle,
    conversation_id: Option<&str>,
    path: String,
) -> Result<PathBuf, String> {
    let conversation_id = normalize_conversation_id(conversation_id)
        .ok_or_else(|| "当前会话尚未创建，不能更换会话工作区".to_string())?;
    let root = validate_workspace_root(path)?;

    let _guard = workspace_store_lock()
        .lock()
        .map_err(|_| "工作区配置锁已损坏".to_string())?;
    let mut store = read_workspace_store(app)?;
    store
        .roots
        .insert(conversation_id, root.display().to_string());
    write_workspace_store(app, &store)?;

    Ok(root)
}

fn validate_workspace_root(path: String) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("请选择有效的工作区目录".to_string());
    }

    let canonical = PathBuf::from(trimmed)
        .canonicalize()
        .map_err(|error| format!("无法解析工作区目录: {}", error))?;
    if !canonical.is_dir() {
        return Err("工作区必须是目录".to_string());
    }

    Ok(canonical)
}

fn normalize_relative_path(path: Option<String>) -> Result<PathBuf, String> {
    let Some(raw_path) = path else {
        return Ok(PathBuf::new());
    };
    let normalized = raw_path.trim().replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(PathBuf::new());
    }

    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err("工作区路径必须是相对路径".to_string());
    }

    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("工作区路径不能越过根目录".to_string());
            }
        }
    }

    Ok(clean)
}

fn resolve_workspace_path(root: &Path, path: Option<String>) -> Result<(PathBuf, String), String> {
    let relative = normalize_relative_path(path)?;
    let target = root.join(&relative);
    let canonical = target
        .canonicalize()
        .map_err(|error| format!("无法读取工作区路径: {}", error))?;
    if !canonical.starts_with(root) {
        return Err("拒绝访问工作区之外的路径".to_string());
    }

    Ok((canonical, relative_to_slash(&relative)))
}

fn relative_to_slash(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn entry_from_path(root: &Path, path: PathBuf) -> Option<WorkspaceEntry> {
    let metadata = std::fs::metadata(&path).ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    if name.starts_with('.') {
        return None;
    }

    let relative = path.strip_prefix(root).ok().map(relative_to_slash)?;
    let kind = if metadata.is_dir() {
        "directory"
    } else {
        "file"
    }
    .to_string();
    let extension = if metadata.is_file() {
        path.extension()
            .map(|ext| ext.to_string_lossy().to_string())
    } else {
        None
    };
    let size = metadata.is_file().then_some(metadata.len());
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());

    Some(WorkspaceEntry {
        name,
        path: path.display().to_string(),
        relative_path: relative,
        kind,
        extension,
        size,
        modified,
    })
}

#[tauri::command]
pub fn workspace_list_directory(
    app: AppHandle,
    conversation_id: Option<String>,
    path: Option<String>,
) -> Result<WorkspaceDirectoryListing, String> {
    let root = workspace_root_for_conversation(&app, conversation_id.as_deref())?;
    list_directory_for_root(root, path)
}

fn list_directory_for_root(
    root: PathBuf,
    path: Option<String>,
) -> Result<WorkspaceDirectoryListing, String> {
    let (target, relative_path) = resolve_workspace_path(&root, path)?;
    if !target.is_dir() {
        return Err("目标路径不是目录".to_string());
    }

    let mut entries = std::fs::read_dir(&target)
        .map_err(|error| format!("读取目录失败: {}", error))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry_from_path(&root, entry.path()))
        .collect::<Vec<_>>();

    entries.sort_by(|a, b| {
        let a_dir = a.kind == "directory";
        let b_dir = b.kind == "directory";
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(WorkspaceDirectoryListing {
        root: root.display().to_string(),
        path: target.display().to_string(),
        relative_path,
        entries,
    })
}

#[tauri::command]
pub fn workspace_set_root(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<WorkspaceDirectoryListing, String> {
    let root = set_workspace_root_for_conversation(&app, conversation_id.as_deref(), path)?;
    list_directory_for_root(root, None)
}

#[tauri::command]
pub fn workspace_read_text_file(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<WorkspaceFileContent, String> {
    let root = workspace_root_for_conversation(&app, conversation_id.as_deref())?;
    let (target, relative_path) = resolve_workspace_path(&root, Some(path))?;
    if !target.is_file() {
        return Err("目标路径不是文件".to_string());
    }

    let metadata =
        std::fs::metadata(&target).map_err(|error| format!("读取文件信息失败: {}", error))?;
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err("文件超过 1MB，暂不在工作区预览中打开".to_string());
    }

    let bytes = std::fs::read(&target).map_err(|error| format!("读取文件失败: {}", error))?;
    let content =
        String::from_utf8(bytes).map_err(|_| "文件不是 UTF-8 文本，暂不支持预览".to_string())?;

    Ok(WorkspaceFileContent {
        path: target.display().to_string(),
        relative_path,
        content,
        size: metadata.len(),
    })
}
