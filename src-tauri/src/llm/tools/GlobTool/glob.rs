use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolDisclosure, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::Tool;
use crate::llm::utils::permissions::protected_path_violation;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    // read_only=true，permission=Some（搜索敏感目录需审批，与 ReadTool/GrepTool 保持一致）
    app_tool(tool, execute_with_app_boxed, true, Some(glob_permission), ToolDisclosure::Core)
}

/// GlobTool 权限检查：搜索路径命中受保护路径时需审批。
/// 攻击场景：Glob(pattern="**/*", path="/Users/victim/.ssh")
fn glob_permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let path = input.get("path").and_then(Value::as_str).unwrap_or("");
    if path.is_empty() {
        return None;
    }
    if protected_path_violation(path).is_err() {
        return Some(ToolPermissionDescriptor {
            signature: format!("glob:sensitive:{}", path),
            preview: format!("搜索敏感路径 {}", path),
            warning: Some("该路径可能包含凭据或密钥".to_string()),
            risk: crate::llm::utils::permissions::RiskLevel::Risky,
        });
    }
    None
}

pub fn tool() -> Tool {
    Tool {
        name: "Glob".into(),
        description: r#"Fast file pattern matching. Supports glob patterns like `**/*.js` or `src/**/*.ts`.

- `pattern`: the glob pattern to match files against.
- `path`: the directory to search in. Defaults to the current workspace directory if not specified.

Returns matching file paths sorted by modification time (most recent first).

Only files are returned, never directories, and version-control directories (`.git`, `.svn`) are always skipped. At most 1000 matches are returned; narrow the pattern if results are truncated."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. If not specified, the current working directory will be used."
                }
            },
            "required": ["pattern"]
        }),
    }
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let raw_pattern = input
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: pattern"))?;

    let base_path = match input.get("path").and_then(Value::as_str) {
        Some(p) => PathBuf::from(p),
        None => crate::command::workspace::workspace_root_for_conversation(app, conversation_id)
            .unwrap_or_else(|_| {
                crate::command::workspace::default_workspace_root(app)
                    .unwrap_or_else(|_| PathBuf::from("."))
            }),
    };

    if !base_path.is_dir() {
        return Err(ToolFailure::new(format!(
            "path is not a directory or does not exist: {}",
            base_path.display()
        )));
    }

    let search_pattern = base_path.join(raw_pattern);
    let pattern_str = search_pattern.to_string_lossy();

    let entries = match glob::glob(&pattern_str) {
        Ok(iter) => iter,
        Err(e) => {
            return Err(ToolFailure::invalid_input(format!("Invalid glob pattern: {}", e)));
        }
    };

    let mut results: Vec<(PathBuf, SystemTime)> = Vec::new();
    let mut scanned = 0usize;

    for entry in entries.flatten() {
        // SCAN_HARD_CAP 保护内存：`**/*` 这类模式在 node_modules / .git 下会命中海量条目。
        if scanned >= SCAN_HARD_CAP {
            break;
        }
        scanned += 1;

        if !entry.is_file() {
            continue;
        }
        // 跳过版本控制目录内部文件。Glob 只返回文件，所以 `pattern="*"` 匹配到的 `.git`
        // 是目录、本来就会被过滤；但 `**/*` 会把 .git 内部成百上千个对象文件全部列出，
        // 既无信息量又爆上下文。这里统一排除。
        if has_ignored_component(&entry) {
            continue;
        }

        let mtime = std::fs::metadata(&entry)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        results.push((entry, mtime));
    }

    results.sort_by(|a, b| b.1.cmp(&a.1));

    if results.is_empty() {
        return Ok(ToolOutcome::text(format!(
            "No files matched pattern: {}",
            raw_pattern
        )));
    }

    // 按 mtime 排序后只保留最近的 MAX_RESULTS 条（结果已按最近修改优先）。
    let truncated = results.len() > MAX_RESULTS;
    results.truncate(MAX_RESULTS);

    let paths: Vec<String> = results
        .into_iter()
        // 归一化 Windows verbatim 前缀（\\?\C:\...），否则同一目录会随参数形式不同
        // 返回两种路径格式，模型据此拼出的路径在后续工具里不可用。
        .map(|(path, _)| crate::command::workspace::display_path_string(&path))
        .collect();

    let mut output = paths.join("\n");
    if truncated {
        output.push_str(&format!(
            "\n\n[truncated: showing {} of {} matches, most recently modified first]",
            MAX_RESULTS, scanned
        ));
    }
    Ok(ToolOutcome::text(output))
}

/// Glob 扫描与返回的条目上限。
const MAX_RESULTS: usize = 1_000;
const SCAN_HARD_CAP: usize = 50_000;

/// 路径中是否含有需要排除的目录组件（版本控制目录）。
fn has_ignored_component(path: &Path) -> bool {
    const IGNORED: [&str; 2] = [".git", ".svn"];
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| IGNORED.contains(&name))
    })
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}
