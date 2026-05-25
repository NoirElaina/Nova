use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

mod editing;
mod patch;

pub use editing::{multi_edit_change, write_file_change, FileEditResult, MultiEditRequest};
pub use patch::{apply_patch_change, patch_paths};

#[derive(Debug, Clone)]
struct FileChangeDraft {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffLine {
    pub kind: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeEntry {
    pub path: String,
    pub absolute_path: String,
    pub change_type: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub diff: Vec<FileDiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeBatch {
    pub id: String,
    pub conversation_id: String,
    pub tool_name: String,
    pub created_at: u64,
    pub reverted: bool,
    pub reverted_at: Option<u64>,
    pub files: Vec<FileChangeEntry>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileChangeStore {
    batches: Vec<FileChangeBatch>,
}

static FILE_CHANGE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const MAX_STORED_BATCHES: usize = 200;

fn file_change_lock() -> &'static Mutex<()> {
    FILE_CHANGE_LOCK.get_or_init(|| Mutex::new(()))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_conversation_id(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__default__")
        .to_string()
}

fn safe_conversation_file_name(conversation_id: &str) -> String {
    conversation_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn store_path(app: &AppHandle, conversation_id: &str) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| {
            path.join("file_changes").join(format!(
                "{}.json",
                safe_conversation_file_name(conversation_id)
            ))
        })
        .map_err(|error| format!("无法读取应用数据目录: {}", error))
}

fn read_store(path: &Path) -> Result<FileChangeStore, String> {
    if !path.exists() {
        return Ok(FileChangeStore::default());
    }
    let text = fs::read_to_string(path).map_err(|error| format!("读取审查记录失败: {}", error))?;
    if text.trim().is_empty() {
        return Ok(FileChangeStore::default());
    }
    serde_json::from_str(&text).map_err(|error| format!("解析审查记录失败: {}", error))
}

fn write_store(path: &Path, store: &FileChangeStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建审查记录目录失败: {}", error))?;
    }
    let text = serde_json::to_string_pretty(store)
        .map_err(|error| format!("序列化审查记录失败: {}", error))?;
    fs::write(path, text).map_err(|error| format!("保存审查记录失败: {}", error))
}

pub fn resolve_tool_path(root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("path is required".to_string());
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve workspace root: {}", error))?;
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return resolve_absolute_tool_path(&root, path, raw_path);
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path cannot leave workspace: {}", raw_path));
            }
        }
    }
    resolve_absolute_tool_path(&root, &root.join(clean), raw_path)
}

fn resolve_absolute_tool_path(root: &Path, path: &Path, raw_path: &str) -> Result<PathBuf, String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("path cannot leave workspace: {}", raw_path));
    }

    if path.exists() {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("failed to resolve path: {}", error))?;
        if !canonical.starts_with(root) {
            return Err(format!("path cannot leave workspace: {}", raw_path));
        }
        return Ok(canonical);
    }

    let mut ancestor = path;
    let mut missing = Vec::<OsString>::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| format!("path cannot be resolved: {}", raw_path))?;
        missing.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("path cannot be resolved: {}", raw_path))?;
    }

    let canonical_ancestor = ancestor
        .canonicalize()
        .map_err(|error| format!("failed to resolve path parent: {}", error))?;
    if !canonical_ancestor.starts_with(root) {
        return Err(format!("path cannot leave workspace: {}", raw_path));
    }

    let mut resolved = canonical_ancestor;
    for part in missing.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

fn path_for_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|relative| {
            relative
                .components()
                .filter_map(|component| match component {
                    Component::Normal(part) => Some(part.to_string_lossy().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn change_type(before: &Option<String>, after: &Option<String>) -> String {
    match (before, after) {
        (None, Some(_)) => "added",
        (Some(_), None) => "deleted",
        _ => "modified",
    }
    .to_string()
}

fn build_entry(root: &Path, draft: FileChangeDraft) -> FileChangeEntry {
    let diff = diff_lines(draft.before.as_deref(), draft.after.as_deref());
    FileChangeEntry {
        path: path_for_display(root, &draft.path),
        absolute_path: draft.path.display().to_string(),
        change_type: change_type(&draft.before, &draft.after),
        before: draft.before,
        after: draft.after,
        diff,
    }
}

fn build_batch(
    root: &Path,
    conversation_id: &str,
    tool_name: &str,
    drafts: Vec<FileChangeDraft>,
) -> Option<FileChangeBatch> {
    let mut seen = BTreeSet::new();
    let files = drafts
        .into_iter()
        .filter(|draft| draft.before != draft.after)
        .filter(|draft| seen.insert(draft.path.clone()))
        .map(|draft| build_entry(root, draft))
        .collect::<Vec<_>>();

    if files.is_empty() {
        return None;
    }

    Some(FileChangeBatch {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        tool_name: tool_name.to_string(),
        created_at: now_millis(),
        reverted: false,
        reverted_at: None,
        files,
    })
}

fn commit_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    tool_name: &str,
    drafts: Vec<FileChangeDraft>,
) -> Result<Option<String>, String> {
    let conversation_id = normalize_conversation_id(conversation_id);
    let Some(batch) = build_batch(root, &conversation_id, tool_name, drafts) else {
        return Ok(None);
    };
    let batch_id = batch.id.clone();
    let path = store_path(app, &conversation_id)?;
    let _guard = file_change_lock()
        .lock()
        .map_err(|_| "审查记录锁已损坏".to_string())?;
    let mut store = read_store(&path)?;
    validate_batch_current_state(root, &batch)?;
    apply_batch_after(root, &batch)?;
    store.batches.push(batch);
    if store.batches.len() > MAX_STORED_BATCHES {
        let drain_count = store.batches.len() - MAX_STORED_BATCHES;
        store.batches.drain(0..drain_count);
    }
    if let Err(error) = write_store(&path, &store) {
        rollback_batch_after(root, store.batches.last().expect("batch was just pushed"))?;
        return Err(error);
    }
    Ok(Some(batch_id))
}

pub fn list_change_batches(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<Vec<FileChangeBatch>, String> {
    let conversation_id = normalize_conversation_id(conversation_id);
    let path = store_path(app, &conversation_id)?;
    let _guard = file_change_lock()
        .lock()
        .map_err(|_| "审查记录锁已损坏".to_string())?;
    let mut batches = read_store(&path)?.batches;
    batches.reverse();
    Ok(batches)
}

pub fn revert_change_batch(
    app: &AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    batch_id: &str,
) -> Result<FileChangeBatch, String> {
    let conversation_id = normalize_conversation_id(conversation_id);
    let path = store_path(app, &conversation_id)?;
    let _guard = file_change_lock()
        .lock()
        .map_err(|_| "审查记录锁已损坏".to_string())?;
    let mut store = read_store(&path)?;
    let batch = store
        .batches
        .iter_mut()
        .find(|batch| batch.id == batch_id)
        .ok_or_else(|| "找不到审查记录".to_string())?;
    apply_revert(root, batch)?;
    let result = batch.clone();
    write_store(&path, &store)?;
    Ok(result)
}

fn apply_revert(root: &Path, batch: &mut FileChangeBatch) -> Result<(), String> {
    if batch.reverted {
        return Err("这次变更已经回退过了".to_string());
    }

    for file in &batch.files {
        let target = PathBuf::from(&file.absolute_path);
        if !target.starts_with(root) {
            return Err(format!("拒绝回退工作区之外的文件: {}", file.path));
        }
        match &file.after {
            Some(expected) => {
                let current = fs::read_to_string(&target)
                    .map_err(|error| format!("读取当前文件失败 {}: {}", file.path, error))?;
                if &current != expected {
                    return Err(format!("文件已被后续修改，不能安全回退: {}", file.path));
                }
            }
            None => {
                if target.exists() {
                    return Err(format!("文件已被后续重新创建，不能安全回退: {}", file.path));
                }
            }
        }
    }

    for file in batch.files.iter().rev() {
        let target = PathBuf::from(&file.absolute_path);
        match &file.before {
            Some(content) => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("创建目录失败 {}: {}", file.path, error))?;
                }
                fs::write(&target, content)
                    .map_err(|error| format!("写回文件失败 {}: {}", file.path, error))?;
            }
            None => {
                if target.exists() {
                    fs::remove_file(&target)
                        .map_err(|error| format!("删除新增文件失败 {}: {}", file.path, error))?;
                }
            }
        }
    }

    batch.reverted = true;
    batch.reverted_at = Some(now_millis());
    Ok(())
}

fn validate_batch_current_state(root: &Path, batch: &FileChangeBatch) -> Result<(), String> {
    for file in &batch.files {
        let target = path_inside_root(root, file)?;
        match &file.before {
            Some(expected) => {
                let current = fs::read_to_string(&target)
                    .map_err(|error| format!("读取当前文件失败 {}: {}", file.path, error))?;
                if &current != expected {
                    return Err(format!("文件在写入前已变化，拒绝覆盖: {}", file.path));
                }
            }
            None => {
                if target.exists() {
                    return Err(format!("新增文件已存在，拒绝覆盖: {}", file.path));
                }
            }
        }
    }
    Ok(())
}

fn apply_batch_after(root: &Path, batch: &FileChangeBatch) -> Result<(), String> {
    let mut applied = Vec::<FileChangeEntry>::new();
    for file in &batch.files {
        if let Err(error) = apply_file_snapshot(root, file, &file.after) {
            let rollback_error = rollback_applied(root, &applied).err();
            return Err(match rollback_error {
                Some(rollback_error) => format!("{}；回滚失败: {}", error, rollback_error),
                None => error,
            });
        }
        applied.push(file.clone());
    }
    Ok(())
}

fn rollback_batch_after(root: &Path, batch: &FileChangeBatch) -> Result<(), String> {
    rollback_applied(root, &batch.files)
}

fn rollback_applied(root: &Path, files: &[FileChangeEntry]) -> Result<(), String> {
    for file in files.iter().rev() {
        apply_file_snapshot(root, file, &file.before)?;
    }
    Ok(())
}

fn apply_file_snapshot(
    root: &Path,
    file: &FileChangeEntry,
    snapshot: &Option<String>,
) -> Result<(), String> {
    let target = path_inside_root(root, file)?;
    match snapshot {
        Some(content) => {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("创建目录失败 {}: {}", file.path, error))?;
            }
            fs::write(&target, content)
                .map_err(|error| format!("写入文件失败 {}: {}", file.path, error))
        }
        None => {
            if target.exists() {
                fs::remove_file(&target)
                    .map_err(|error| format!("删除文件失败 {}: {}", file.path, error))?;
            }
            Ok(())
        }
    }
}

fn path_inside_root(root: &Path, file: &FileChangeEntry) -> Result<PathBuf, String> {
    let target = PathBuf::from(&file.absolute_path);
    if !target.starts_with(root) {
        return Err(format!("拒绝访问工作区之外的文件: {}", file.path));
    }
    Ok(target)
}

fn diff_lines(before: Option<&str>, after: Option<&str>) -> Vec<FileDiffLine> {
    TextDiff::from_lines(before.unwrap_or_default(), after.unwrap_or_default())
        .iter_all_changes()
        .map(|change| FileDiffLine {
            kind: match change.tag() {
                ChangeTag::Delete => "remove",
                ChangeTag::Insert => "add",
                ChangeTag::Equal => "context",
            }
            .to_string(),
            old_line: change.old_index().map(|index| index + 1),
            new_line: change.new_index().map(|index| index + 1),
            text: change
                .value()
                .trim_end_matches(&['\r', '\n'][..])
                .to_string(),
        })
        .collect()
}
