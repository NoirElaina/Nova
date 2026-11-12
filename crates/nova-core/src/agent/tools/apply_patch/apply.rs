//! 执行已解析的补丁集合并写回工作区文件系统。

use std::path::{Path, PathBuf};
use serde_json::{json, Value};

use crate::agent::utils::file_io::{
    invalidate_file_content_cache, read_file_meta, resolve_tool_path, write_file_with_meta,
    FileEncoding, FileMeta, LineEnding,
};
use crate::agent::utils::paths::{canonical_starts_with, resolve_absolute_path_for_write};
use super::file_update::derive_new_contents_from_chunks;
use super::parser::Hunk;

#[derive(Debug, Default)]
pub struct PatchExecutionReport {
    pub files_modified: Vec<String>,
    pub files_added: Vec<String>,
    pub files_deleted: Vec<String>,
    pub diffs: Vec<String>,
}

impl PatchExecutionReport {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": true,
            "modified": self.files_modified,
            "added": self.files_added,
            "deleted": self.files_deleted,
            "diff": self.diffs.join("\n"),
        })
    }
}

/// 执行全部 Hunk 操作，保证操作原子性与缓存一致性。
enum PreparedOp {
    Add {
        resolved: PathBuf,
        display: String,
        contents: String,
    },
    Delete {
        resolved: PathBuf,
        display: String,
    },
    Update {
        source: PathBuf,
        dest: PathBuf,
        display: String,
        move_display: Option<String>,
        new_content: String,
        meta: FileMeta,
        unified_diff: String,
    },
}

/// 执行全部 Hunk 操作：先预检与内存演算所有变更，通过后再落盘，避免前一半写入后失败留脏状态。
pub async fn execute_patch_hunks(
    workspace_root: &Path,
    hunks: &[Hunk],
) -> Result<PatchExecutionReport, String> {
    if hunks.is_empty() {
        return Err("No hunks to apply in patch".to_string());
    }

    // 阶段 1：预检全部 hunk 的路径边界、源文件读取与上下文匹配，不修改磁盘。
    let mut prepared = Vec::with_capacity(hunks.len());
    for hunk in hunks {
        match hunk {
            Hunk::AddFile { path, contents } => {
                let resolved = resolve_path(workspace_root, path)?;
                prepared.push(PreparedOp::Add {
                    resolved,
                    display: path.display().to_string(),
                    contents: contents.clone(),
                });
            }
            Hunk::DeleteFile { path } => {
                let resolved = resolve_path(workspace_root, path)?;
                prepared.push(PreparedOp::Delete {
                    resolved,
                    display: path.display().to_string(),
                });
            }
            Hunk::UpdateFile {
                path,
                move_path,
                chunks,
            } => {
                let source = resolve_path(workspace_root, path)?;
                let (original_content, meta) = read_file_meta(&source)
                    .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
                let display = path.display().to_string();
                let diff_outcome = derive_new_contents_from_chunks(&original_content, &display, chunks)
                    .map_err(|e| format!("Patch application error in {display}: {e}"))?;
                let (dest, move_display) = if let Some(target_move) = move_path {
                    (
                        resolve_path(workspace_root, target_move)?,
                        Some(target_move.display().to_string()),
                    )
                } else {
                    (source.clone(), None)
                };
                prepared.push(PreparedOp::Update {
                    source,
                    dest,
                    display,
                    move_display,
                    new_content: diff_outcome.new_content,
                    meta,
                    unified_diff: diff_outcome.unified_diff,
                });
            }
        }
    }

    // 阶段 2：全部预检通过，按序写盘。
    let mut report = PatchExecutionReport::default();
    for op in prepared {
        match op {
            PreparedOp::Add {
                resolved,
                display,
                contents,
            } => {
                let meta = FileMeta {
                    encoding: FileEncoding::Utf8,
                    line_ending: LineEnding::Lf,
                };
                write_file_with_meta(&resolved, &contents, &meta)
                    .map_err(|e| format!("Failed to create {display}: {e}"))?;
                report.files_added.push(display.clone());
                report.diffs.push(format!("Added file: {display}\n+ {} bytes", contents.len()));
            }
            PreparedOp::Delete { resolved, display } => {
                if resolved.exists() {
                    std::fs::remove_file(&resolved)
                        .map_err(|e| format!("Failed to delete {display}: {e}"))?;
                    invalidate_file_content_cache(&resolved);
                }
                report.files_deleted.push(display.clone());
                report.diffs.push(format!("Deleted file: {display}"));
            }
            PreparedOp::Update {
                source,
                dest,
                display,
                move_display,
                new_content,
                meta,
                unified_diff,
            } => {
                write_file_with_meta(&dest, &new_content, &meta)
                    .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
                if let Some(target_disp) = move_display {
                    if source != dest && source.exists() {
                        let _ = std::fs::remove_file(&source);
                        invalidate_file_content_cache(&source);
                    }
                    report.files_modified.push(format!("{display} -> {target_disp}"));
                } else {
                    report.files_modified.push(display);
                }
                invalidate_file_content_cache(&dest);
                report.diffs.push(unified_diff);
            }
        }
    }

    Ok(report)
}

/// 把补丁里的路径解析成工作区内的绝对路径。
///
/// 三条硬约束（缺一就会写出工作区）：
/// 1. 相对路径以**会话工作区**为基准，而不是进程 cwd；
/// 2. 拒绝 `..` 与 UNC；
/// 3. canonicalize（或最近已存在祖先 canonicalize）后必须仍在工作区根内。
fn resolve_path(workspace_root: &Path, raw: &Path) -> Result<PathBuf, String> {
    use std::path::Component;

    let raw_str = raw.to_string_lossy();
    if raw_str.starts_with("\\\\") || raw_str.starts_with("//") {
        return Err(format!(
            "patch path must not be a UNC path: {}",
            raw.display()
        ));
    }
    if raw
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!(
            "patch path must not contain '..': {}",
            raw.display()
        ));
    }

    let base = if workspace_root.is_absolute() {
        workspace_root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("failed to resolve current dir: {e}"))?
            .join(workspace_root)
    };
    let root = resolve_absolute_path_for_write(&base.to_string_lossy(), "workspace_root")?;

    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    };
    let resolved = resolve_tool_path(&candidate.to_string_lossy())?;

    if !canonical_starts_with(&resolved, &root) {
        return Err(format!(
            "patch path escapes the conversation workspace: {} (workspace: {})",
            resolved.display(),
            root.display()
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::apply_patch::parser::parse_patch;

    #[test]
    fn resolve_path_accepts_relative_inside_workspace() {
        let root = std::env::temp_dir();
        let resolved = resolve_path(&root, Path::new("nova_patch_resolve_test.txt")).unwrap();
        let canonical_root = root.canonicalize().unwrap();
        assert!(canonical_starts_with(&resolved, &canonical_root));
    }

    #[test]
    fn resolve_path_works_for_uncreated_workspace_and_blocks_outside_abs_path() {
        let missing_ws = std::env::temp_dir().join("nova_uncreated_ws_boundary_check");
        let inside = resolve_path(&missing_ws, Path::new("sub/new.txt"))
            .expect("relative path inside uncreated workspace must resolve cleanly");
        assert!(inside.ends_with(Path::new("sub").join("new.txt")));

        let outside = std::env::temp_dir().join("nova_outside_target.txt");
        assert!(
            resolve_path(&missing_ws, &outside).is_err(),
            "absolute path outside workspace must be rejected"
        );
    }

    #[test]
    fn resolve_path_rejects_parent_dir_and_unc() {
        let root = std::env::temp_dir();
        assert!(resolve_path(&root, Path::new("../escape.txt")).is_err());
        assert!(resolve_path(&root, Path::new("//server/share/x.txt")).is_err());
    }

    #[tokio::test]
    async fn execute_patch_hunks_preflights_all_hunks_before_writing() {
        let ws = std::env::temp_dir().join(format!("nova-patch-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();

        let bad_patch = "*** Begin Patch\n*** Add File: should_not_exist.txt\n+hello\n*** Delete File: ../escape.txt\n*** End Patch";
        let parsed = parse_patch(bad_patch).unwrap();
        let err = execute_patch_hunks(&ws, &parsed.hunks).await.unwrap_err();
        assert!(err.contains(".."));
        assert!(
            !ws.join("should_not_exist.txt").exists(),
            "hunk 1 must not hit disk when hunk 2 fails preflight"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }
}
