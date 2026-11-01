//! 原子写入：tempfile + fsync + rename + EXDEV/EBUSY fallback
//!
//! 写入过程中崩溃只会留下孤儿 tmp 文件，目标文件保持上一个完整版本。
//! 保留原文件权限（chmod temp 为原 mode 后再 rename）。
//! 解析 symlink 到真实路径后写入，保留 symlink 本身。
//! rename 失败（EXDEV/EBUSY/Windows sharing violation）回退到非原子 write + fsync。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, PersistError};

/// 将 `content` 以 UTF-8 原子写入 `target`，保留原文件权限与 symlink。
pub fn write_str(target: &Path, content: &str) -> io::Result<()> {
    write_bytes(target, content.as_bytes())
}

/// 将 `content` 字节原子写入 `target`，保留原文件权限与 symlink。
pub fn write_bytes(target: &Path, content: &[u8]) -> io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    // 解析 symlink 到真实路径，保留 symlink 本身
    let real_target = resolve_symlink(target);

    // 记录原文件权限（仅 Unix 有意义），rename 前应用到 temp 文件。
    #[cfg(unix)]
    let target_mode = fs::metadata(&real_target)
        .ok()
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode()
        });

    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(content)?;
    tmp.flush()?;
    // fsync 不可用（部分 Windows 文件系统）时静默忽略
    let _ = tmp.as_file().sync_all();

    // 应用原文件权限到 temp（仅 Unix）
    #[cfg(unix)]
    if let Some(mode) = target_mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(mode))?;
    }

    match tmp.persist(&real_target) {
        Ok(_) => Ok(()),
        Err(PersistError { error, file: _ }) => {
            // EXDEV/EBUSY/Windows sharing violation：回退到 copy + fsync
            // tmp file 在 PersistError::file 中，离开作用域自动删除
            if is_cross_device_or_busy(&error) {
                fallback_write(&real_target, content)
            } else {
                Err(error)
            }
        }
    }
}

fn resolve_symlink(target: &Path) -> PathBuf {
    // 只在 target 本身是 symlink 时解析，避免对不存在文件 canonicalize 失败
    if fs::symlink_metadata(target)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf())
    } else {
        target.to_path_buf()
    }
}

fn is_cross_device_or_busy(err: &io::Error) -> bool {
    match err.raw_os_error() {
        Some(18) => true,                            // EXDEV (Unix)
        Some(17) if cfg!(windows) => true,           // ERROR_NOT_SAME_DEVICE
        Some(16) => true,                            // EBUSY (Linux)
        Some(32) if cfg!(windows) => true,           // ERROR_SHARING_VIOLATION
        _ => false,
    }
}

fn fallback_write(target: &Path, content: &[u8]) -> io::Result<()> {
    // EXDEV fallback：直接 fs::write + fsync，非原子但跨设备唯一可行方案
    fs::write(target, content)?;
    let f = fs::File::open(target)?;
    let _ = f.sync_all();
    Ok(())
}
