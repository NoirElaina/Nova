use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

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

fn workspace_root() -> Result<PathBuf, String> {
    std::env::current_dir()
        .map_err(|error| format!("无法读取当前工作区目录: {}", error))?
        .canonicalize()
        .map_err(|error| format!("无法解析当前工作区目录: {}", error))
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
pub fn workspace_list_directory(path: Option<String>) -> Result<WorkspaceDirectoryListing, String> {
    let root = workspace_root()?;
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
pub fn workspace_read_text_file(path: String) -> Result<WorkspaceFileContent, String> {
    let root = workspace_root()?;
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
