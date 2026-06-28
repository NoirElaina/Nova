// Git 服务层：纯 git 命令封装 + codex 风格实时 diff 收集 + 会话工作区包装。
//
// 审查页改为 codex 风格：实时 git diff HEAD + untracked，不再持久化 batch、不再回退。

use std::path::Path;
use std::process::Command;

use tauri::AppHandle;

fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 git {:?}: {}", args, e))?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "git {:?} 失败 (exit={:?}): {}",
            args,
            output.status.code(),
            stderr
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// 仅检查工作区是否已经是一个 git 仓库（存在 `.git`）。
pub fn is_repo_initialized(root: &Path) -> bool {
    root.join(".git").exists()
}

/// 返回当前分支名（detached HEAD 时返回短 SHA）。仓库未初始化或查询失败返回 None。
pub fn current_branch(root: &Path) -> Option<String> {
    if !is_repo_initialized(root) {
        return None;
    }
    let name = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "HEAD" {
        run_git(root, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        Some(trimmed.to_string())
    }
}

/// 当前路径是 git worktree（而非主仓库）时返回 worktree 名（取目录名）。主仓库或未初始化返回 None。
pub fn current_worktree_name(root: &Path) -> Option<String> {
    if !is_repo_initialized(root) {
        return None;
    }
    let common = run_git(root, &["rev-parse", "--git-common-dir"]).ok()?;
    let current = run_git(root, &["rev-parse", "--git-dir"]).ok()?;
    if common.trim() == current.trim() {
        return None;
    }
    root.file_name().map(|n| n.to_string_lossy().to_string())
}

/// 显式把 target_root 初始化为 git 仓库。已是仓库则啥也不做。
pub fn ensure_repo(root: &Path) -> Result<(), String> {
    if root.join(".git").exists() {
        return Ok(());
    }
    run_git(root, &["init", "--quiet"])?;
    let _ = run_git(root, &["config", "user.name", "Nova"]);
    let _ = run_git(root, &["config", "user.email", "nova@local"]);
    Ok(())
}

// ============================================================================
// codex 风格：实时 git diff HEAD + untracked 文件
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffLine {
    pub kind: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    pub path: String,
    pub absolute_path: String,
    pub change_type: String,
    pub additions: usize,
    pub deletions: usize,
    pub diff: Vec<FileDiffLine>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiff {
    pub files: Vec<WorkspaceFileChange>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

const MAX_TEXT_FILE_BYTES: u64 = 1024 * 1024;

/// 解析 `git diff --numstat` 输出，返回 (path, additions, deletions, is_binary)。
fn read_numstat(root: &Path, args: &[&str]) -> Result<Vec<(String, usize, usize, bool)>, String> {
    let out = run_git(root, args)?;
    let mut rows = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let add = match parts.next() {
            Some("-") | None => continue,
            Some(v) => v,
        };
        let del = parts.next().unwrap_or("0");
        let path = parts.next().unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }
        let is_binary = !add.chars().all(|c| c.is_ascii_digit());
        let additions = if is_binary { 0 } else { add.parse::<usize>().unwrap_or(0) };
        let deletions = if is_binary { 0 } else { del.parse::<usize>().unwrap_or(0) };
        rows.push((path, additions, deletions, is_binary));
    }
    Ok(rows)
}

/// 解析 `git diff --name-status` 输出，返回 path -> status 字符的映射。
fn read_name_status(root: &Path, args: &[&str]) -> Result<std::collections::HashMap<String, String>, String> {
    let out = run_git(root, args)?;
    let mut map = std::collections::HashMap::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let status = parts.next().unwrap_or("M").to_string();
        let path = parts.next().unwrap_or("").to_string();
        if !path.is_empty() {
            map.insert(path, status);
        }
    }
    Ok(map)
}

/// 解析单文件 unified diff 输出为 FileDiffLine 列表。
fn parse_unified_diff(text: &str) -> Vec<FileDiffLine> {
    let mut out = Vec::new();
    let mut old_line: Option<usize> = None;
    let mut new_line: Option<usize> = None;
    let mut in_hunk = false;
    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with("diff ") || line.starts_with("index ") || line.starts_with("--- ")
            || line.starts_with("+++ ") || line.starts_with("new file") || line.is_empty()
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@") {
            let h = rest.split("@@").next().unwrap_or("");
            let parts: Vec<&str> = h.split_whitespace().collect();
            if let Some(old_part) = parts.iter().find(|p| p.starts_with('-')) {
                let a = old_part[1..].split(',').next().unwrap_or("1");
                old_line = a.parse::<usize>().ok();
            }
            if let Some(new_part) = parts.iter().find(|p| p.starts_with('+')) {
                let c = new_part[1..].split(',').next().unwrap_or("1");
                new_line = c.parse::<usize>().ok();
            }
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        let (kind, text_body) = match line.chars().next() {
            Some('+') => ("add", &line[1..]),
            Some('-') => ("remove", &line[1..]),
            Some(' ') => ("context", &line[1..]),
            Some('\\') => continue,
            _ => ("context", line),
        };
        let cur_old = old_line;
        let cur_new = new_line;
        out.push(FileDiffLine {
            kind: kind.to_string(),
            old_line: cur_old,
            new_line: cur_new,
            text: text_body.to_string(),
        });
        match kind {
            "add" => {
                if let Some(n) = new_line.as_mut() {
                    *n += 1;
                }
            }
            "remove" => {
                if let Some(n) = old_line.as_mut() {
                    *n += 1;
                }
            }
            _ => {
                if let Some(n) = old_line.as_mut() {
                    *n += 1;
                }
                if let Some(n) = new_line.as_mut() {
                    *n += 1;
                }
            }
        }
    }
    out
}

fn read_file_diff(root: &Path, args: &[&str]) -> Vec<FileDiffLine> {
    match run_git(root, args) {
        Ok(s) => parse_unified_diff(&s),
        Err(_) => Vec::new(),
    }
}

/// 收集工作区相对 HEAD 的全部改动（含 untracked 文件），按 codex 风格实现。
///
/// 步骤：
/// 1. `git diff HEAD --numstat` + `--name-status` 拿到已跟踪文件的改动（仓库尚无提交时跳过）。
/// 2. `git ls-files --others --exclude-standard` 拿到 untracked 文件列表。
/// 3. 对每个 untracked 文件直接读内容生成 diff（避免 Windows NUL 设备名问题）。
pub fn collect_workspace_diff(root: &Path) -> Result<WorkspaceDiff, String> {
    if !is_repo_initialized(root) {
        return Ok(WorkspaceDiff {
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
        });
    }

    let mut files = Vec::new();
    let mut total_add = 0usize;
    let mut total_del = 0usize;

    // 1. 已跟踪文件相对 HEAD 的改动。仓库刚 init 还没有任何 commit 时 HEAD 不存在，
    //    此时所有文件都还没被跟踪，跳过此步直接走 untracked 路径。
    let has_head = run_git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    if has_head {
        let numstat = read_numstat(root, &["diff", "HEAD", "--numstat", "--no-renames", "--no-color"])?;
        let name_status = read_name_status(root, &["diff", "HEAD", "--name-status", "--no-renames", "--no-color"])?;

        for (path, additions, deletions, is_binary) in &numstat {
            let status = name_status.get(path).cloned().unwrap_or_else(|| "M".to_string());
            let change_type = match status.as_str() {
                "A" => "added",
                "D" => "deleted",
                _ => "modified",
            }
            .to_string();
            total_add += additions;
            total_del += deletions;

            let diff = if *is_binary {
                vec![FileDiffLine {
                    kind: "context".to_string(),
                    old_line: None,
                    new_line: None,
                    text: "(二进制文件)".to_string(),
                }]
            } else {
                read_file_diff(root, &[
                    "diff",
                    "--no-prefix",
                    "--no-color",
                    "--unified=3",
                    "--no-textconv",
                    "--no-ext-diff",
                    "HEAD",
                    "--",
                    path,
                ])
            };

            let abs = root.join(path).display().to_string();
            files.push(WorkspaceFileChange {
                path: path.clone(),
                absolute_path: abs,
                change_type,
                additions: *additions,
                deletions: *deletions,
                diff,
            });
        }
    }

    // 2. untracked 文件。直接读取文件内容生成 diff，
    //    避免 Windows 下 `git diff --no-index NUL <file>` 因 NUL 是保留设备名而失败。
    let untracked_out = run_git(root, &["ls-files", "--others", "--exclude-standard"])?;
    for file in untracked_out.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let file_path = root.join(file);
        // 跳过超大文件，避免读取爆内存。
        if let Ok(meta) = std::fs::metadata(&file_path) {
            if meta.len() > MAX_TEXT_FILE_BYTES {
                continue;
            }
        }
        // 读不到（含二进制）就跳过，不在此处伪造空 diff。
        let content = match std::fs::read_to_string(&file_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut diff_lines = Vec::new();
        for (i, line) in content.lines().enumerate() {
            diff_lines.push(FileDiffLine {
                kind: "add".to_string(),
                old_line: None,
                new_line: Some(i + 1),
                text: line.to_string(),
            });
        }
        let additions = diff_lines.len();
        total_add += additions;
        let abs = file_path.display().to_string();
        files.push(WorkspaceFileChange {
            path: file.to_string(),
            absolute_path: abs,
            change_type: "added".to_string(),
            additions,
            deletions: 0,
            diff: diff_lines,
        });
    }

    Ok(WorkspaceDiff {
        files,
        total_additions: total_add,
        total_deletions: total_del,
    })
}

// ============================================================================
// 会话工作区包装层：AppHandle → 路径解析 + git 状态/diff/init
// ============================================================================

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepoStatus {
    pub initialized: bool,
    pub path: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

fn repo_status_for_root(root: &Path) -> GitRepoStatus {
    let initialized = is_repo_initialized(root);
    let branch = if initialized { current_branch(root) } else { None };
    let worktree = if initialized { current_worktree_name(root) } else { None };
    GitRepoStatus {
        initialized,
        path: crate::command::workspace::display_path_string(root),
        branch,
        worktree,
    }
}

/// 查询会话工作区的 git 初始化状态，供前端决定按钮文案/可见性。
pub fn get_conversation_repo_status(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<GitRepoStatus, String> {
    let repo_root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    Ok(repo_status_for_root(&repo_root))
}

/// 基于显式路径查询 git 状态。供 EnvironmentBar 在无会话时使用。
pub fn get_repo_status_by_path(workspace_path: &str) -> Result<GitRepoStatus, String> {
    let trimmed = workspace_path.trim();
    if trimmed.is_empty() {
        return Err("工作区路径为空".to_string());
    }
    let root = std::path::PathBuf::from(trimmed)
        .canonicalize()
        .map_err(|e| format!("无法解析工作区路径: {}", e))?;
    if !root.is_dir() {
        return Err("工作区路径不是目录".to_string());
    }
    Ok(repo_status_for_root(&root))
}

/// 收集会话工作区的 git diff（已跟踪 + untracked）。
pub fn collect_conversation_diff(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<WorkspaceDiff, String> {
    let repo_root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    collect_workspace_diff(&repo_root)
}

/// 用户在审查页点击「初始化 Git」时调用。把会话工作区显式 init 成 git 仓库。
/// 返回 (是否新建了 .git, 仓库根绝对路径)。
pub fn init_conversation_repo(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<(bool, String), String> {
    let repo_root = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    let already = is_repo_initialized(&repo_root);
    ensure_repo(&repo_root)?;
    let path_str = crate::command::workspace::display_path_string(&repo_root);
    Ok((!already, path_str))
}

/// 生成会话工作区的 git 状态摘要文本（用于注入模型上下文）。
/// 轻量级：只读取分支名、dirty 文件路径+变更类型、最近 5 条 commit oneline，
/// 不收集完整 diff（避免与大 diff 收集重复成本）。
/// 仓库未初始化时返回 None。
pub fn workspace_git_status_summary(root: &Path) -> Option<String> {
    if !is_repo_initialized(root) {
        return None;
    }

    let branch = current_branch(root).unwrap_or_else(|| "HEAD".to_string());

    // dirty 文件：用 porcelain 格式，轻量且包含 staged+unstaged+untracked。
    // 输出形如 " M src/main.rs"、"?? new.txt"、"A  staged.txt"。
    let dirty = run_git(root, &["status", "--porcelain"]).ok();
    let dirty_lines: Vec<&str> = dirty
        .as_ref()
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).collect())
        .unwrap_or_default();

    // 最近 5 条 commit oneline。
    let log = run_git(root, &["log", "--oneline", "-5", "--no-decorate"]).ok();
    let log_lines: Vec<&str> = log
        .as_ref()
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).collect())
        .unwrap_or_default();

    let mut out = format!("Branch: {}\n", branch);

    if dirty_lines.is_empty() {
        out.push_str("Working tree: clean\n");
    } else {
        out.push_str(&format!("Working tree: {} changed file(s)\n", dirty_lines.len()));
        // 限制注入条数，避免超大改动爆上下文。
        let max_show = 30usize;
        for line in dirty_lines.iter().take(max_show) {
            // porcelain 行格式: "XY path"，保留原样以便 agent 看到变更类型。
            out.push_str(&format!("  {}\n", line));
        }
        if dirty_lines.len() > max_show {
            out.push_str(&format!(
                "  ...and {} more\n",
                dirty_lines.len() - max_show
            ));
        }
    }

    if !log_lines.is_empty() {
        out.push_str("Recent commits:\n");
        for line in &log_lines {
            out.push_str(&format!("  {}\n", line));
        }
    }

    Some(out)
}
