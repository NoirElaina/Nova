use super::{commit_change_batch, path_for_display, resolve_tool_path, FileChangeDraft};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct MultiEditRequest {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub expected_replacements: usize,
}

#[derive(Debug, Clone)]
pub struct FileEditResult {
    pub files: Vec<String>,
    pub change_batch_id: Option<String>,
}

pub async fn write_file_change(
    app: &AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    raw_path: &str,
    content: &str,
) -> Result<FileEditResult, String> {
    let target = resolve_tool_path(root, raw_path)?;
    let before = if target.exists() {
        Some(
            fs::read_to_string(&target)
                .map_err(|error| format!("Cannot safely capture existing file: {}", error))?,
        )
    } else {
        None
    };

    commit_drafts(
        app,
        conversation_id,
        root,
        "write_file",
        vec![FileChangeDraft {
            path: target,
            before,
            after: Some(content.to_string()),
        }],
    )
    .await
}

pub async fn multi_edit_change(
    app: &AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    edits: Vec<MultiEditRequest>,
) -> Result<FileEditResult, String> {
    if edits.is_empty() {
        return Err("multi_edit requires at least one edit".to_string());
    }

    let mut pending = BTreeMap::<_, String>::new();
    let mut originals = BTreeMap::<_, String>::new();

    for edit in edits {
        if edit.old_string.is_empty() {
            return Err(format!("{} old_string must not be empty", edit.path));
        }
        if edit.expected_replacements == 0 {
            return Err(format!(
                "{} expected_replacements must be greater than 0",
                edit.path
            ));
        }

        let target = resolve_tool_path(root, &edit.path)?;
        let mut content = match pending.get(&target) {
            Some(content) => content.clone(),
            None => {
                let content = fs::read_to_string(&target)
                    .map_err(|error| format!("Error reading {}: {}", edit.path, error))?;
                originals.entry(target.clone()).or_insert(content.clone());
                content
            }
        };

        let count = content.matches(&edit.old_string).count();
        if count != edit.expected_replacements {
            return Err(format!(
                "{} expected {} replacement(s), found {}",
                edit.path, edit.expected_replacements, count
            ));
        }

        content = content.replace(&edit.old_string, &edit.new_string);
        pending.insert(target, content);
    }

    let drafts = pending
        .into_iter()
        .map(|(path, after)| FileChangeDraft {
            before: originals.get(&path).cloned(),
            path,
            after: Some(after),
        })
        .collect::<Vec<_>>();

    commit_drafts(app, conversation_id, root, "multi_edit", drafts).await
}

pub(super) async fn commit_drafts(
    app: &AppHandle,
    conversation_id: Option<&str>,
    root: &Path,
    tool_name: &str,
    drafts: Vec<FileChangeDraft>,
) -> Result<FileEditResult, String> {
    let files = drafts
        .iter()
        .filter(|draft| draft.before != draft.after)
        .map(|draft| path_for_display(root, &draft.path))
        .collect::<Vec<_>>();
    let change_batch_id =
        commit_change_batch(app, conversation_id, root, tool_name, drafts).await?;
    let files = if change_batch_id.is_some() {
        files
    } else {
        Vec::new()
    };

    Ok(FileEditResult {
        files,
        change_batch_id,
    })
}
