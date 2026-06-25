use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{RwLock, OnceLock};
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

// 会话工作区路径的进程内缓存。
// 数据库 conversations.workspace_path 是唯一事实来源；该缓存仅供同步热路径读取。
// 工作区路径在会话创建后不可变，故缓存不会过期。
static CONVERSATION_WORKSPACE_CACHE: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

fn conversation_workspace_cache() -> &'static RwLock<HashMap<String, String>> {
    CONVERSATION_WORKSPACE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

// 将单个会话的工作区路径写入缓存。由 create_conversation 调用。
pub fn cache_conversation_workspace(conversation_id: &str, workspace_path: &str) {
    if let Ok(mut cache) = conversation_workspace_cache().write() {
        cache.insert(conversation_id.to_string(), workspace_path.to_string());
    }
}

// 从数据库批量刷新缓存。由 list_conversations 调用。
pub async fn refresh_workspace_cache(entries: &[(String, Option<String>)]) {
    if let Ok(mut cache) = conversation_workspace_cache().write() {
        cache.clear();
        for (id, path) in entries {
            if let Some(p) = path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                cache.insert(id.clone(), p.to_string());
            }
        }
    }
}

fn normalize_conversation_id(conversation_id: Option<&str>) -> Option<String> {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

// 内置默认工作区目录：app_data/workspace。新会话未指定工作区时使用。
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

// 解析会话的工作区根目录（同步，读缓存）。缓存未命中时回退到内置默认工作区。
pub fn workspace_root_for_conversation(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<PathBuf, String> {
    let Some(conversation_id) = normalize_conversation_id(conversation_id) else {
        return default_workspace_root(app);
    };

    let cached = conversation_workspace_cache()
        .read()
        .ok()
        .and_then(|cache| cache.get(&conversation_id).cloned());

    let Some(root) = cached else {
        return default_workspace_root(app);
    };

    let canonical = PathBuf::from(&root)
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
    workspace_root_for_conversation(app, conversation_id).map(|path| display_path_string(&path))
}

pub fn display_path_text(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
        const VERBATIM_PREFIX: &str = r"\\?\";

        if let Some(rest) = path.strip_prefix(VERBATIM_UNC_PREFIX) {
            return format!(r"\\{}", rest);
        }
        if let Some(rest) = path.strip_prefix(VERBATIM_PREFIX) {
            return rest.to_string();
        }
    }

    path.to_string()
}

pub fn display_path_string(path: &Path) -> String {
    display_path_text(&path.display().to_string())
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
        path: display_path_string(&path),
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
        root: display_path_string(&root),
        path: display_path_string(&target),
        relative_path,
        entries,
    })
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
        path: display_path_string(&target),
        relative_path,
        content,
        size: metadata.len(),
    })
}
