use crate::llm::services::file_changes::{
    commit_drafts, read_file_utf8, resolve_tool_path, FileChangeDraft, FileEditResult,
};
use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Tool registration
// ---------------------------------------------------------------------------

pub(super) fn registrations() -> Vec<ToolRegistration> {
    vec![app_tool(
        apply_patch_tool,
        apply_patch_with_app,
        false,
        Some(apply_patch_permission),
    )]
}

fn apply_patch_tool() -> Tool {
    Tool {
        name: "apply_patch".into(),
        description: r#"Use `apply_patch` to edit files. Pass the raw patch string directly as the tool input.

## Format

*** Begin Patch
[ one or more file operations ]
*** End Patch

Each operation starts with one of three headers:

*** Add File: <path> — create a new file. Every following line MUST start with +.
*** Delete File: <path> — remove an existing file. Nothing follows.
*** Update File: <path> — patch an existing file in place.

May be followed by *** Move to: <new path> to rename the file.
Then one or more "hunks", each introduced by @@ (optionally followed by a description).
Within a hunk each line starts with:

- (space) context line — must match file content (fuzzy matching: trim whitespace, normalize Unicode)
- + added line
- - removed line

## Context Rules

Show 3 lines of context above and below each change. If changes are close, merge them into one hunk.
If 3 lines isn't enough to uniquely identify the location, use @@ to specify the enclosing class/function:

@@ class BaseClass
@@   def method():
-  old_code
+  new_code

## Rules

- Paths MUST be absolute (e.g., /home/user/project/file.py)
- Every line after *** Add File: MUST start with +
- Context lines use fuzzy matching (handles whitespace/indentation differences)
- Use *** End of File to mark changes at the end of a file

## Example

*** Begin Patch
*** Add File: /home/user/project/hello.txt
+Hello world
*** Update File: /home/user/project/src/app.py
@@ def greet():
-  print("Hi")
+  print("Hello, world!")
*** Delete File: /home/user/project/obsolete.txt
*** End Patch"#
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Raw patch text starting with *** Begin Patch and ending with *** End Patch"
                }
            },
            "required": ["patch"]
        }),
    }
}

fn apply_patch_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let patch = match &input {
            Value::String(s) => s.as_str(),
            Value::Object(obj) => obj.get("patch").and_then(Value::as_str).unwrap_or(""),
            _ => {
                return Err(ToolFailure::invalid_input(
                    "apply_patch requires raw patch text or {patch: ...}",
                ))
            }
        };
        match apply_patch_change(&app, conversation_id.as_deref(), patch).await {
            Ok(result) => result_json(result),
            Err(error) => Err(ToolFailure::new(error)),
        }
    })
}

fn apply_patch_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let patch = match input {
        Value::String(s) => s.as_str(),
        Value::Object(obj) => obj.get("patch").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    };
    match patch_paths(patch) {
        Ok(paths) => describe_paths_permission("apply_patch", "文件补丁", paths),
        Err(error) => Some(ToolPermissionDescriptor {
            signature: "apply_patch:<invalid>".to_string(),
            preview: "文件补丁（apply_patch）：补丁格式无效".to_string(),
            warning: Some(error),
            needs_approval: false,
        }),
    }
}

fn describe_paths_permission(
    tool_name: &str,
    preview_label: &str,
    paths: Vec<String>,
) -> Option<ToolPermissionDescriptor> {
    let unique = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>();

    if unique.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:<empty>", tool_name),
            preview: format!("{}（{}）：未提供文件路径", preview_label, tool_name),
            warning: Some("目标路径为空，无法执行。".to_string()),
            needs_approval: false,
        });
    }

    let paths = unique.iter().cloned().collect::<Vec<_>>();
    let mut warning = None;
    let mut needs_approval = false;
    for path in &paths {
        if let Some(descriptor) = crate::llm::utils::permissions::describe_file_write_permission(
            tool_name,
            preview_label,
            "path",
            &json!({ "path": path }),
        ) {
            if descriptor.needs_approval {
                needs_approval = true;
            }
            if warning.is_none() {
                warning = descriptor.warning;
            }
        }
    }

    let preview_paths = paths
        .iter()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if paths.len() > 4 {
        format!(" 等 {} 个文件", paths.len())
    } else {
        format!("{} 个文件", paths.len())
    };

    Some(ToolPermissionDescriptor {
        signature: format!(
            "{}:{}",
            tool_name,
            paths
                .iter()
                .map(|path| path.replace('/', "\\").to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("|")
        ),
        preview: format!(
            "{}（{}）：{}{}",
            preview_label, tool_name, preview_paths, suffix
        ),
        warning,
        needs_approval,
    })
}

fn result_json(result: FileEditResult) -> Result<ToolOutcome, ToolFailure> {
    let changed_files = result.files.len();
    Ok(ToolOutcome::json(json!({
        "ok": true,
        "files": result.files,
        "changed_files": changed_files,
        "change_batch_id": result.change_batch_id
    })))
}

// ---------------------------------------------------------------------------
// Patch core: parse, apply hunks, find subsequence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum PatchOperation {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<PatchHunk>,
    },
}

#[derive(Debug, Clone)]
struct PatchHunk {
    lines: Vec<PatchLine>,
    is_end_of_file: bool,
}

#[derive(Debug, Clone)]
enum PatchLine {
    Context(String),
    Add(String),
    Remove(String),
}

fn patch_paths(patch: &str) -> Result<Vec<String>, String> {
    parse_patch(patch).map(|operations| {
        operations
            .into_iter()
            .flat_map(|operation| match operation {
                PatchOperation::Add { path, .. } | PatchOperation::Delete { path } => {
                    vec![path]
                }
                PatchOperation::Update { path, move_to, .. } => match move_to {
                    Some(new_path) => vec![path, new_path],
                    None => vec![path],
                },
            })
            .collect()
    })
}

async fn apply_patch_change(
    app: &AppHandle,
    conversation_id: Option<&str>,
    patch: &str,
) -> Result<FileEditResult, String> {
    let drafts = patch_to_drafts(patch)?;
    commit_drafts(app, conversation_id, "apply_patch", drafts).await
}

fn patch_to_drafts(patch: &str) -> Result<Vec<FileChangeDraft>, String> {
    let operations = parse_patch(patch)?;
    let mut pending: BTreeMap<_, Option<String>> = BTreeMap::new();
    let mut originals: BTreeMap<_, Option<String>> = BTreeMap::new();

    for operation in operations {
        match operation {
            PatchOperation::Add { path, content } => {
                let target = resolve_tool_path(&path)?;
                if target.exists() || pending.contains_key(&target) {
                    return Err(format!("Add File target already exists: {}", path));
                }
                originals.entry(target.clone()).or_insert(None);
                pending.insert(target, Some(content));
            }
            PatchOperation::Delete { path } => {
                let target = resolve_tool_path(&path)?;
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
                            read_file_utf8(&target)
                                .map_err(|error| format!("Error reading {}: {}", path, error))?,
                        ),
                    };
                    originals.insert(target.clone(), before);
                }
                pending.insert(target, None);
            }
            PatchOperation::Update {
                path,
                move_to,
                hunks,
            } => {
                let target = resolve_tool_path(&path)?;
                if !originals.contains_key(&target) {
                    originals.insert(
                        target.clone(),
                        Some(
                            read_file_utf8(&target)
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

                match move_to {
                    Some(new_path) => {
                        let dest = resolve_tool_path(&new_path)?;
                        if dest != target
                            && (dest.exists() || pending.get(&dest).is_some_and(Option::is_some))
                        {
                            return Err(format!("Move target already exists: {}", new_path));
                        }
                        pending.insert(target, None);
                        originals.entry(dest.clone()).or_insert(None);
                        pending.insert(dest, Some(next));
                    }
                    None => {
                        pending.insert(target, Some(next));
                    }
                }
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
    let normalized = patch.trim().replace("\r\n", "\n");
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

            let mut move_to = None;
            if index < lines.len() {
                if let Some(new_path) = lines[index].strip_prefix("*** Move to: ") {
                    move_to = Some(new_path.trim().to_string());
                    index += 1;
                }
            }

            let mut hunks = Vec::new();
            while index < lines.len() && !is_patch_boundary(lines[index]) {
                // @@ context header or direct diff lines
                if lines[index].starts_with("@@") {
                    index += 1;
                }
                let mut hunk_lines = Vec::new();
                let mut eof = false;
                while index < lines.len()
                    && !lines[index].starts_with("@@")
                    && !is_patch_boundary(lines[index])
                {
                    let raw = lines[index];
                    if raw == "*** End of File" {
                        eof = true;
                        index += 1;
                        break;
                    }
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
                hunks.push(PatchHunk { lines: hunk_lines, is_end_of_file: eof });
            }
            if hunks.is_empty() {
                return Err(format!("Update File has no hunks: {}", path));
            }
            operations.push(PatchOperation::Update {
                path: path.trim().to_string(),
                move_to,
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
        || line.starts_with("*** Move to: ")
}

fn apply_hunks(original: &str, hunks: &[PatchHunk]) -> Result<String, String> {
    let eol = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = original.replace("\r\n", "\n");
    let had_final_newline = normalized.ends_with('\n');
    let total_lines = if had_final_newline {
        normalized.split('\n').count() - 1
    } else {
        normalized.split('\n').count()
    };
    let mut lines = normalized
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    if had_final_newline {
        lines.pop();
    }

    let mut cursor = 0;
    for (hunk_idx, hunk) in hunks.iter().enumerate() {
        let hunk_num = hunk_idx + 1;
        let old_lines: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(text) | PatchLine::Remove(text) => Some(text.clone()),
                PatchLine::Add(_) => None,
            })
            .collect();
        let new_lines: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(text) | PatchLine::Add(text) => Some(text.clone()),
                PatchLine::Remove(_) => None,
            })
            .collect();

        if old_lines.is_empty() && !new_lines.is_empty() {
            // Pure addition: insert at cursor or end of file.
            let insert_at = cursor.min(lines.len());
            lines.splice(insert_at..insert_at, new_lines.clone());
            cursor = insert_at + new_lines.len();
            continue;
        }

        if old_lines.is_empty() {
            return Err(format!(
                "hunk {hunk_num} has no context or removed lines"
            ));
        }

        // Try to find old_lines in the file, with EOF hint support.
        let mut found = seek_sequence(&lines, &old_lines, cursor, hunk.is_end_of_file);

        // Retry: if trailing empty line (representing final newline) prevents match,
        // strip it from both old_lines and new_lines and retry.
        let mut retry_old: Option<&[String]> = None;
        let mut retry_new: Option<&[String]> = None;
        if found.is_none() && old_lines.last().is_some_and(String::is_empty) {
            let trimmed_old = &old_lines[..old_lines.len() - 1];
            let trimmed_new = if new_lines.last().is_some_and(String::is_empty) {
                &new_lines[..new_lines.len() - 1]
            } else {
                new_lines.as_slice()
            };
            found = seek_sequence(&lines, trimmed_old, cursor, hunk.is_end_of_file);
            if found.is_some() {
                retry_old = Some(trimmed_old);
                retry_new = Some(trimmed_new);
            }
        }

        let (effective_old, effective_new) = match (retry_old, retry_new) {
            (Some(o), Some(n)) => (o.to_vec(), n.to_vec()),
            _ => (old_lines.clone(), new_lines.clone()),
        };

        let Some(start) = found else {
            let preview = effective_old
                .iter()
                .take(3)
                .map(|l| format!("  {}", l))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!(
                "patch context not found in hunk {hunk_num}/{total} \
                 (file has {total_lines} lines, searching from line {cursor_pos})\n\
                 Expected lines:\n{preview}",
                total = hunks.len(),
                cursor_pos = cursor + 1,
            ));
        };

        let end = start + effective_old.len();
        lines.splice(start..end, effective_new.clone());
        cursor = start + effective_new.len();
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

/// Attempt to find `pattern` within `lines` starting at or after `start`.
/// Uses four progressive strictness levels:
///   1. Exact byte match
///   2. Right-strip only (trim trailing whitespace)
///   3. Full trim (both leading and trailing whitespace)
///   4. Unicode normalisation (fancy quotes/dashes/spaces → ASCII equivalents)
///
/// When `eof` is true, the search starts from the end of the file so that
/// patterns intended to match file endings are applied there first.
fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }

    let search_start = if eof && lines.len() >= pattern.len() {
        lines.len() - pattern.len()
    } else {
        start
    };
    let end = lines.len().saturating_sub(pattern.len());

    // Pass 1: exact match.
    for i in search_start..=end {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }

    // Pass 2: rstrip – ignore trailing whitespace.
    for i in search_start..=end {
        if pattern
            .iter()
            .enumerate()
            .all(|(j, p)| lines[i + j].trim_end() == p.trim_end())
        {
            return Some(i);
        }
    }

    // Pass 3: full trim – ignore leading and trailing whitespace.
    for i in search_start..=end {
        if pattern
            .iter()
            .enumerate()
            .all(|(j, p)| lines[i + j].trim() == p.trim())
        {
            return Some(i);
        }
    }

    // Pass 4: Unicode normalisation – map fancy punctuation to ASCII.
    fn normalise(s: &str) -> String {
        s.trim()
            .chars()
            .map(|c| match c {
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
                | '\u{2212}' => '-',
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
                '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
                | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
                | '\u{3000}' => ' ',
                other => other,
            })
            .collect()
    }

    for i in search_start..=end {
        if pattern
            .iter()
            .enumerate()
            .all(|(j, p)| normalise(&lines[i + j]) == normalise(p))
        {
            return Some(i);
        }
    }

    None
}
