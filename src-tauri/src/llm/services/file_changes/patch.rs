use super::editing::{commit_drafts, FileEditResult};
use super::{resolve_tool_path, FileChangeDraft};
use std::collections::BTreeMap;
use std::path::Path;
use tauri::AppHandle;

#[derive(Debug, Clone)]
enum PatchOperation {
    Add { path: String, content: String },
    Delete { path: String },
    Update { path: String, hunks: Vec<PatchHunk> },
}

#[derive(Debug, Clone)]
struct PatchHunk {
    lines: Vec<PatchLine>,
}

#[derive(Debug, Clone)]
enum PatchLine {
    Context(String),
    Add(String),
    Remove(String),
}

pub fn patch_paths(patch: &str) -> Result<Vec<String>, String> {
    parse_patch(patch).map(|operations| {
        operations
            .into_iter()
            .map(|operation| match operation {
                PatchOperation::Add { path, .. }
                | PatchOperation::Delete { path }
                | PatchOperation::Update { path, .. } => path,
            })
            .collect()
    })
}

pub fn apply_patch_change(
    app: &AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    patch: &str,
) -> Result<FileEditResult, String> {
    let drafts = patch_to_drafts(root, patch)?;
    commit_drafts(app, conversation_id, root, "apply_patch", drafts)
}

fn patch_to_drafts(root: &Path, patch: &str) -> Result<Vec<FileChangeDraft>, String> {
    let operations = parse_patch(patch)?;
    let mut pending: BTreeMap<_, Option<String>> = BTreeMap::new();
    let mut originals: BTreeMap<_, Option<String>> = BTreeMap::new();

    for operation in operations {
        match operation {
            PatchOperation::Add { path, content } => {
                let target = resolve_tool_path(root, &path)?;
                if target.exists() || pending.contains_key(&target) {
                    return Err(format!("Add File target already exists: {}", path));
                }
                originals.entry(target.clone()).or_insert(None);
                pending.insert(target, Some(content));
            }
            PatchOperation::Delete { path } => {
                let target = resolve_tool_path(root, &path)?;
                if pending.get(&target).is_some_and(Option::is_none) {
                    return Err(format!("Delete File target already deleted: {}", path));
                }
                if !target.exists() && !pending.contains_key(&target) {
                    return Err(format!("Delete File target does not exist: {}", path));
                }
                if !originals.contains_key(&target) {
                    let before = match pending.get(&target) {
                        Some(Some(content)) => Some(content.clone()),
                        Some(None) => None,
                        None => Some(
                            std::fs::read_to_string(&target)
                                .map_err(|error| format!("Error reading {}: {}", path, error))?,
                        ),
                    };
                    originals.insert(target.clone(), before);
                }
                pending.insert(target, None);
            }
            PatchOperation::Update { path, hunks } => {
                let target = resolve_tool_path(root, &path)?;
                if !originals.contains_key(&target) {
                    originals.insert(
                        target.clone(),
                        Some(
                            std::fs::read_to_string(&target)
                                .map_err(|error| format!("Error reading {}: {}", path, error))?,
                        ),
                    );
                }
                let current = match pending.get(&target) {
                    Some(Some(content)) => content.clone(),
                    Some(None) => return Err(format!("Cannot update deleted file: {}", path)),
                    None => originals
                        .get(&target)
                        .and_then(Clone::clone)
                        .ok_or_else(|| format!("Error reading {}", path))?,
                };
                let next = apply_hunks(&current, &hunks)
                    .map_err(|error| format!("{}: {}", path, error))?;
                pending.insert(target, Some(next));
            }
        }
    }

    Ok(pending
        .into_iter()
        .map(|(path, after)| FileChangeDraft {
            before: originals.get(&path).cloned().unwrap_or(None),
            path,
            after,
        })
        .collect())
}

fn parse_patch(patch: &str) -> Result<Vec<PatchOperation>, String> {
    let normalized = patch.replace("\r\n", "\n");
    let mut lines = normalized.split('\n').collect::<Vec<_>>();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.first().copied() != Some("*** Begin Patch") {
        return Err("patch must start with *** Begin Patch".to_string());
    }

    let mut index = 1;
    let mut operations = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line == "*** End Patch" {
            if index != lines.len() - 1 {
                return Err("patch has content after *** End Patch".to_string());
            }
            return Ok(operations);
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut added = Vec::new();
            while index < lines.len() && !is_patch_boundary(lines[index]) {
                let raw = lines[index];
                let Some(text) = raw.strip_prefix('+') else {
                    return Err(format!("Add File line must start with +: {}", raw));
                };
                added.push(text.to_string());
                index += 1;
            }
            let content = if added.is_empty() {
                String::new()
            } else {
                format!("{}\n", added.join("\n"))
            };
            operations.push(PatchOperation::Add {
                path: path.trim().to_string(),
                content,
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(PatchOperation::Delete {
                path: path.trim().to_string(),
            });
            index += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut hunks = Vec::new();
            while index < lines.len() && !is_patch_boundary(lines[index]) {
                if !lines[index].starts_with("@@") {
                    return Err(format!(
                        "Update File expected @@ hunk, got: {}",
                        lines[index]
                    ));
                }
                index += 1;
                let mut hunk_lines = Vec::new();
                while index < lines.len()
                    && !lines[index].starts_with("@@")
                    && !is_patch_boundary(lines[index])
                {
                    let raw = lines[index];
                    let line = if let Some(text) = raw.strip_prefix(' ') {
                        PatchLine::Context(text.to_string())
                    } else if let Some(text) = raw.strip_prefix('+') {
                        PatchLine::Add(text.to_string())
                    } else if let Some(text) = raw.strip_prefix('-') {
                        PatchLine::Remove(text.to_string())
                    } else {
                        return Err(format!("Invalid hunk line: {}", raw));
                    };
                    hunk_lines.push(line);
                    index += 1;
                }
                if hunk_lines.is_empty() {
                    return Err("empty patch hunk".to_string());
                }
                hunks.push(PatchHunk { lines: hunk_lines });
            }
            if hunks.is_empty() {
                return Err(format!("Update File has no hunks: {}", path));
            }
            operations.push(PatchOperation::Update {
                path: path.trim().to_string(),
                hunks,
            });
            continue;
        }

        return Err(format!("unknown patch directive: {}", line));
    }

    Err("patch must end with *** End Patch".to_string())
}

fn is_patch_boundary(line: &str) -> bool {
    line == "*** End Patch"
        || line.starts_with("*** Add File: ")
        || line.starts_with("*** Update File: ")
        || line.starts_with("*** Delete File: ")
}

fn apply_hunks(original: &str, hunks: &[PatchHunk]) -> Result<String, String> {
    let eol = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = original.replace("\r\n", "\n");
    let had_final_newline = normalized.ends_with('\n');
    let mut lines = normalized
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    if had_final_newline {
        lines.pop();
    }

    let mut cursor = 0;
    for hunk in hunks {
        let old_lines = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(text) | PatchLine::Remove(text) => Some(text.clone()),
                PatchLine::Add(_) => None,
            })
            .collect::<Vec<_>>();
        let new_lines = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(text) | PatchLine::Add(text) => Some(text.clone()),
                PatchLine::Remove(_) => None,
            })
            .collect::<Vec<_>>();

        if old_lines.is_empty() {
            return Err("hunk must include context or removed lines".to_string());
        }

        let Some(relative) = find_subsequence(&lines[cursor..], &old_lines) else {
            return Err("patch context not found".to_string());
        };
        let start = cursor + relative;
        let end = start + old_lines.len();
        lines.splice(start..end, new_lines.clone());
        cursor = start + new_lines.len();
    }

    let mut content = lines.join("\n");
    if had_final_newline {
        content.push('\n');
    }
    if eol == "\r\n" {
        Ok(content.replace('\n', "\r\n"))
    } else {
        Ok(content)
    }
}

fn find_subsequence(haystack: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
