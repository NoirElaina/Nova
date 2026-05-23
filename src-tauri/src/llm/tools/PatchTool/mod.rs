use crate::llm::tools::{app_tool, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use tauri::AppHandle;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![
        app_tool(
            apply_patch_tool,
            execute_sync_stub,
            apply_patch_with_app,
            false,
            Some(apply_patch_permission),
        ),
        app_tool(
            multi_edit_tool,
            execute_sync_stub,
            multi_edit_with_app,
            false,
            Some(multi_edit_permission),
        ),
    ]
}

fn execute_sync_stub(_input: Value) -> String {
    json!({ "ok": false, "error": "File patch tools require AppHandle-aware execution." })
        .to_string()
}

fn apply_patch_tool() -> Tool {
    Tool {
        name: "apply_patch".into(),
        description: "Apply a structured multi-file patch. Prefer this for code edits because it validates context before writing and returns the affected files.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Patch text using *** Begin Patch / *** Update File / *** Add File / *** Delete File / @@ hunks / *** End Patch"
                }
            },
            "required": ["patch"]
        }),
    }
}

fn multi_edit_tool() -> Tool {
    Tool {
        name: "multi_edit".into(),
        description: "Apply multiple exact string replacements after validating every edit. Prefer apply_patch for code changes; use multi_edit for small repeated exact replacements.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Workspace-relative or absolute file path" },
                            "old_string": { "type": "string", "description": "Exact string to replace" },
                            "new_string": { "type": "string", "description": "Replacement string" },
                            "expected_replacements": { "type": "integer", "description": "Exact number of occurrences to replace. Defaults to 1." }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        }),
    }
}

fn apply_patch_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let root = match crate::command::workspace::workspace_root_for_conversation(
            &app,
            conversation_id.as_deref(),
        ) {
            Ok(root) => root,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
        execute_apply_patch_at_root(&root, input)
    })
}

fn multi_edit_with_app(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        let root = match crate::command::workspace::workspace_root_for_conversation(
            &app,
            conversation_id.as_deref(),
        ) {
            Ok(root) => root,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
        execute_multi_edit_at_root(&root, input)
    })
}

fn apply_patch_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let patch = input
        .get("patch")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match parse_patch_paths(patch) {
        Ok(paths) => describe_paths_permission("apply_patch", "文件补丁", paths),
        Err(error) => Some(ToolPermissionDescriptor {
            signature: "apply_patch:<invalid>".to_string(),
            preview: "文件补丁（apply_patch）：补丁格式无效".to_string(),
            warning: Some(error),
            needs_approval: false,
        }),
    }
}

fn multi_edit_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let paths = input
        .get("edits")
        .and_then(Value::as_array)
        .map(|edits| {
            edits
                .iter()
                .filter_map(|edit| edit.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    describe_paths_permission("multi_edit", "批量编辑", paths)
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

    let preview_paths = paths.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
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

#[derive(Debug, Clone)]
struct MultiEdit {
    path: String,
    old_string: String,
    new_string: String,
    expected_replacements: usize,
}

fn execute_apply_patch_at_root(root: &Path, input: Value) -> String {
    result_json(apply_patch_at_root(root, input))
}

fn execute_multi_edit_at_root(root: &Path, input: Value) -> String {
    result_json(multi_edit_at_root(root, input))
}

fn result_json(result: Result<Vec<String>, String>) -> String {
    match result {
        Ok(files) => {
            json!({ "ok": true, "files": files, "changed_files": files.len() }).to_string()
        }
        Err(error) => json!({ "ok": false, "error": error }).to_string(),
    }
}

fn apply_patch_at_root(root: &Path, input: Value) -> Result<Vec<String>, String> {
    let patch = input
        .get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| "apply_patch requires patch".to_string())?;
    let operations = parse_patch(patch)?;
    let mut pending: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut changed = BTreeSet::new();

    for operation in operations {
        match operation {
            PatchOperation::Add { path, content } => {
                let target = resolve_path(root, &path)?;
                if target.exists() || pending.contains_key(&target) {
                    return Err(format!("Add File target already exists: {}", path));
                }
                pending.insert(target, Some(content));
                changed.insert(path);
            }
            PatchOperation::Delete { path } => {
                let target = resolve_path(root, &path)?;
                if pending.get(&target).is_some_and(Option::is_none) {
                    return Err(format!("Delete File target already deleted: {}", path));
                }
                if !target.exists() && !pending.contains_key(&target) {
                    return Err(format!("Delete File target does not exist: {}", path));
                }
                pending.insert(target, None);
                changed.insert(path);
            }
            PatchOperation::Update { path, hunks } => {
                let target = resolve_path(root, &path)?;
                let current = match pending.get(&target) {
                    Some(Some(content)) => content.clone(),
                    Some(None) => return Err(format!("Cannot update deleted file: {}", path)),
                    None => fs::read_to_string(&target)
                        .map_err(|error| format!("Error reading {}: {}", path, error))?,
                };
                let next = apply_hunks(&current, &hunks)
                    .map_err(|error| format!("{}: {}", path, error))?;
                pending.insert(target, Some(next));
                changed.insert(path);
            }
        }
    }

    for (path, content) in pending {
        match content {
            Some(content) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("Error creating parent directory: {}", error))?;
                }
                fs::write(&path, content)
                    .map_err(|error| format!("Error writing {}: {}", path.display(), error))?;
            }
            None => {
                fs::remove_file(&path)
                    .map_err(|error| format!("Error deleting {}: {}", path.display(), error))?;
            }
        }
    }

    Ok(changed.into_iter().collect())
}

fn multi_edit_at_root(root: &Path, input: Value) -> Result<Vec<String>, String> {
    let edits = parse_multi_edits(&input)?;
    let mut pending = BTreeMap::<PathBuf, String>::new();
    let mut changed = BTreeSet::new();

    for edit in edits {
        let target = resolve_path(root, &edit.path)?;
        let mut content = match pending.get(&target) {
            Some(content) => content.clone(),
            None => fs::read_to_string(&target)
                .map_err(|error| format!("Error reading {}: {}", edit.path, error))?,
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
        changed.insert(edit.path);
    }

    for (path, content) in pending {
        fs::write(&path, content)
            .map_err(|error| format!("Error writing {}: {}", path.display(), error))?;
    }

    Ok(changed.into_iter().collect())
}

fn parse_multi_edits(input: &Value) -> Result<Vec<MultiEdit>, String> {
    let edits = input
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| "multi_edit requires edits".to_string())?;
    if edits.is_empty() {
        return Err("multi_edit requires at least one edit".to_string());
    }

    edits
        .iter()
        .enumerate()
        .map(|(index, edit)| {
            let path = required_string(edit, "path")
                .map_err(|error| format!("edits[{}].{}", index, error))?;
            let old_string = required_string(edit, "old_string")
                .map_err(|error| format!("edits[{}].{}", index, error))?;
            let new_string = edit
                .get("new_string")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{}].new_string is required", index))?
                .to_string();
            let expected_replacements = edit
                .get("expected_replacements")
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize;
            if expected_replacements == 0 {
                return Err(format!(
                    "edits[{}].expected_replacements must be greater than 0",
                    index
                ));
            }
            Ok(MultiEdit {
                path,
                old_string,
                new_string,
                expected_replacements,
            })
        })
        .collect()
}

fn required_string(input: &Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} is required", key))
}

fn parse_patch_paths(patch: &str) -> Result<Vec<String>, String> {
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
    let mut content = original.replace("\r\n", "\n");
    let mut cursor = 0;

    for hunk in hunks {
        let old_lines = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(text) | PatchLine::Remove(text) => Some(text.as_str()),
                PatchLine::Add(_) => None,
            })
            .collect::<Vec<_>>();
        let new_lines = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(text) | PatchLine::Add(text) => Some(text.as_str()),
                PatchLine::Remove(_) => None,
            })
            .collect::<Vec<_>>();
        if old_lines.is_empty() {
            return Err("hunk must include context or removed lines".to_string());
        }

        let old_text = old_lines.join("\n");
        let new_text = new_lines.join("\n");
        let Some(relative) = content[cursor..].find(&old_text) else {
            return Err("patch context not found".to_string());
        };
        let start = cursor + relative;
        let end = start + old_text.len();
        content.replace_range(start..end, &new_text);
        cursor = start + new_text.len();
    }

    if eol == "\r\n" {
        Ok(content.replace('\n', "\r\n"))
    } else {
        Ok(content)
    }
}

fn resolve_path(root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("path is required".to_string());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
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
    Ok(root.join(clean))
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn test_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("nova_patch_tool_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn read_json(output: String) -> Value {
        serde_json::from_str(&output).unwrap()
    }

    fn write_file(root: &Path, path: &str, content: &str) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    #[test]
    fn apply_patch_updates_file_with_context() {
        let root = test_root();
        write_file(&root, "src/example.txt", "alpha\nbeta\ngamma\n");
        let patch = "*** Begin Patch\n*** Update File: src/example.txt\n@@\n alpha\n-beta\n+delta\n gamma\n*** End Patch";

        let output = read_json(super::execute_apply_patch_at_root(
            &root,
            json!({ "patch": patch }),
        ));

        assert_eq!(output["ok"], true);
        assert_eq!(
            fs::read_to_string(root.join("src/example.txt")).unwrap(),
            "alpha\ndelta\ngamma\n"
        );
    }

    #[test]
    fn apply_patch_rejects_missing_context_without_writing() {
        let root = test_root();
        write_file(&root, "example.txt", "alpha\nbeta\n");
        let patch = "*** Begin Patch\n*** Update File: example.txt\n@@\n missing\n-beta\n+delta\n*** End Patch";

        let output = read_json(super::execute_apply_patch_at_root(
            &root,
            json!({ "patch": patch }),
        ));

        assert_eq!(output["ok"], false);
        assert_eq!(
            fs::read_to_string(root.join("example.txt")).unwrap(),
            "alpha\nbeta\n"
        );
    }

    #[test]
    fn multi_edit_replaces_expected_occurrence_count() {
        let root = test_root();
        write_file(&root, "example.txt", "foo foo\n");

        let output = read_json(super::execute_multi_edit_at_root(
            &root,
            json!({
                "edits": [
                    {
                        "path": "example.txt",
                        "old_string": "foo",
                        "new_string": "bar",
                        "expected_replacements": 2
                    }
                ]
            }),
        ));

        assert_eq!(output["ok"], true);
        assert_eq!(
            fs::read_to_string(root.join("example.txt")).unwrap(),
            "bar bar\n"
        );
    }

    #[test]
    fn multi_edit_is_atomic_when_later_edit_fails() {
        let root = test_root();
        write_file(&root, "a.txt", "alpha\n");
        write_file(&root, "b.txt", "beta\n");

        let output = read_json(super::execute_multi_edit_at_root(
            &root,
            json!({
                "edits": [
                    {
                        "path": "a.txt",
                        "old_string": "alpha",
                        "new_string": "changed",
                        "expected_replacements": 1
                    },
                    {
                        "path": "b.txt",
                        "old_string": "missing",
                        "new_string": "changed",
                        "expected_replacements": 1
                    }
                ]
            }),
        ));

        assert_eq!(output["ok"], false);
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "alpha\n");
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "beta\n");
    }
}
