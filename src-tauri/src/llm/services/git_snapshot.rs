use std::path::Path;
use std::process::Command;

const NOVA_REF_PREFIX: &str = "refs/nova-snapshots/";
const EMPTY_TREE_SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

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



/// 把 target_root 当成 git 仓库准备就绪。已是仓库则啥也不做；不是则 `git init` 兜底。
pub fn ensure_repo(root: &Path) -> Result<(), String> {
    if root.join(".git").exists() {
        return Ok(());
    }
    run_git(root, &["init", "--quiet"])?;
    // 关掉隐式用户 commit 钩子检查，避免中文路径/权限钩子让快照失败。
    let _ = run_git(root, &["config", "user.name", "Nova"]);
    let _ = run_git(root, &["config", "user.email", "nova@local"]);
    Ok(())
}

/// 在工作区当前状态下创建一个隐藏 ref 快照。不动 HEAD、不动 index、不动工作区。
/// 返回 commit SHA。空工作区（无任何变化）时返回 None。
pub fn create_snapshot(root: &Path, ref_name: &str) -> Result<Option<String>, String> {
    // stash create -u 把工作区改动 + index 改动 + untracked 文件打包成一个 stash commit。
    // 不污染用户 index，也不动工作区。
    let stash_sha = match run_git(root, &["stash", "create", "--include-untracked"]) {
        Ok(sha) if !sha.is_empty() => sha,
        // 空字符串表示没有任何改动，没有快照可建。
        Ok(_) => return Ok(None),
        // stash create 在刚 init、HEAD 不存在的空仓库里会报错：当作"无改动"处理。
        Err(err) if err.contains("not a valid object name") || err.contains("ambiguous") => {
            // 兜底：直接把整个工作区根 tree 写进来作为空快照。
            return fallback_empty_snapshot(root, ref_name);
        }
        Err(err) => return Err(err),
    };

    let ref_full = format!("{NOVA_REF_PREFIX}{ref_name}");
    run_git(root, &["update-ref", &ref_full, &stash_sha])?;
    Ok(Some(stash_sha))
}

fn fallback_empty_snapshot(root: &Path, ref_name: &str) -> Result<Option<String>, String> {
    // 没 HEAD 时直接用空树占位；后续 diff 会把整个 worktree 视为新增。
    let tree_sha = run_git(root, &["mktree"])?;
    if tree_sha.is_empty() {
        return Ok(None);
    }
    let commit_sha = run_git(root, &[
        "commit-tree",
        &tree_sha,
        "-m",
        "nova snapshot (no-head)",
    ])?;
    let ref_full = format!("{NOVA_REF_PREFIX}{ref_name}");
    run_git(root, &["update-ref", &ref_full, &commit_sha])?;
    Ok(Some(commit_sha))
}

fn tree_of(root: &Path, commit_sha: &str) -> Result<String, String> {
    run_git(root, &["rev-parse", &format!("{commit_sha}^{{tree}}")])
}

/// 用 stash 重置工作区到该 commit 的 tree，并清理 untracked，等价"回到那个时刻"。
pub fn revert_to_snapshot(root: &Path, commit_sha: &str) -> Result<(), String> {
    let tree = tree_of(root, commit_sha)?;
    // read-tree -u --reset：index + 工作区重置为该 tree（不动 HEAD）。
    run_git(root, &["read-tree", "-u", "--reset", &tree])?;
    // clean -fd：删除该 tree 之外的 untracked 文件（保留 .gitignore 的）。
    run_git(root, &["clean", "-fd", "--quiet"])?;
    Ok(())
}

#[derive(Debug)]
struct NumstatRow {
    additions: usize,
    deletions: usize,
    path: String,
    is_binary: bool,
}

fn read_numstat(root: &Path, old_tree: &str, new_tree: &str) -> Result<Vec<NumstatRow>, String> {
    let out = run_git(
        root,
        &[
            "diff",
            "--numstat",
            "--no-renames",
            "--no-color",
            old_tree,
            new_tree,
        ],
    )?;
    let mut rows = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // 形如：  "12    3    src/foo.rs"
        // 或二进制："-     -    binary.png"
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
        rows.push(NumstatRow {
            additions,
            deletions,
            path,
            is_binary,
        });
    }
    Ok(rows)
}

fn read_name_status(
    root: &Path,
    old_tree: &str,
    new_tree: &str,
) -> Result<std::collections::HashMap<String, String>, String> {
    let out = run_git(
        root,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            "--no-color",
            old_tree,
            new_tree,
        ],
    )?;
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

// 单文件 unified diff -> Vec<FileDiffLine>
fn read_file_diff(root: &Path, old_tree: &str, new_tree: &str, path: &str) -> Vec<FileDiffLine> {
    let out = match run_git(
        root,
        &[
            "diff",
            "--no-prefix",
            "--no-color",
            "--unified=3",
            "--",
            old_tree,
            new_tree,
            path,
        ],
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    parse_unified_diff(&out)
}

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
pub struct FileChangeEntry {
    pub path: String,
    pub absolute_path: String,
    pub change_type: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub diff: Vec<FileDiffLine>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeBatchSummary {
    pub id: String,
    pub conversation_id: String,
    pub tool_name: String,
    pub created_at: u64,
    pub reverted: bool,
    pub reverted_at: Option<u64>,
    pub file_count: usize,
    pub additions: usize,
    pub deletions: usize,
    pub paths: Vec<String>,
}

/// 解析 git unified patch 整体输出（可能含多文件），返回所有行扁平列表，
/// 不带 hunk 头与 diff 头。用于单文件场景。
fn parse_unified_diff(text: &str) -> Vec<FileDiffLine> {
    let mut out = Vec::new();
    // hunk 头形如 "@@ -a,b +c,d @@ ..."
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
            // @@ -a,b +c,d @@
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
        // 逐行
        let (kind, text_body) = match line.chars().next() {
            Some('+') => ("add", &line[1..]),
            Some('-') => ("remove", &line[1..]),
            Some(' ') => ("context", &line[1..]),
            Some('\\') => {
                // "\ No newline at end of file" 注释，跳过
                continue;
            }
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

pub fn diff_snapshots(
    root: &Path,
    old_commit: Option<&str>,
    new_commit: Option<&str>,
) -> Result<(Vec<FileChangeEntry>, usize, usize), String> {
    let old_tree = match old_commit {
        Some(sha) => tree_of(root, sha)?,
        None => EMPTY_TREE_SHA.to_string(),
    };
    let new_tree = match new_commit {
        Some(sha) => tree_of(root, sha)?,
        None => EMPTY_TREE_SHA.to_string(),
    };
    if old_tree == new_tree {
        return Ok((Vec::new(), 0, 0));
    }

    let name_status = read_name_status(root, &old_tree, &new_tree)?;
    let numstat = read_numstat(root, &old_tree, &new_tree)?;

    let mut files = Vec::new();
    let mut total_add = 0usize;
    let mut total_del = 0usize;
    for row in &numstat {
        let status = name_status.get(&row.path).cloned().unwrap_or_else(|| "M".to_string());
        let change_type = match status.as_str() {
            "A" => "added",
            "D" => "deleted",
            _ => "modified",
        }
        .to_string();
        total_add += row.additions;
        total_del += row.deletions;
        let diff = if row.is_binary {
            vec![FileDiffLine {
                kind: "context".to_string(),
                old_line: None,
                new_line: None,
                text: "(二进制文件)".to_string(),
            }]
        } else {
            read_file_diff(root, &old_tree, &new_tree, &row.path)
        };
        let abs = root.join(&row.path).display().to_string();
        files.push(FileChangeEntry {
            path: row.path.clone(),
            absolute_path: abs,
            change_type,
            before: None,
            after: None,
            diff,
        });
    }
    Ok((files, total_add, total_del))
}