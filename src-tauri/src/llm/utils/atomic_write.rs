//! 原子写入：tempfile + fsync + rename + EXDEV/EBUSY fallback
//!
//! 移植自 hermes-agent utils.py atomic_replace。
//! Nova 是单进程桌面应用，无跨进程写竞争；crash 安全靠 temp+rename 模式保证：
//! 写入过程中崩溃只会留下孤儿 tmp 文件，目标文件保持上一个完整版本。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, PersistError};

/// 将 `content` 以 UTF-8 原子写入 `target`。
///
/// 父目录不存在会自动创建。若 `target` 是符号链接，写入会被解析到真实路径
/// （保留 symlink 本身，对齐 hermes GitHub #16743 修复）。
pub fn write_str(target: &Path, content: &str) -> io::Result<()> {
    write_bytes(target, content.as_bytes())
}

/// 将 `content` 字节原子写入 `target`。
pub fn write_bytes(target: &Path, content: &[u8]) -> io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(content)?;
    tmp.flush()?;
    // fsync 不可用（部分 Windows 文件系统）时静默忽略
    let _ = tmp.as_file().sync_all();

    // 解析 symlink 到真实路径，保留 symlink 本身
    let real_target = resolve_symlink(target);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nova_atomic_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_write_new_file() {
        let dir = tmp_dir();
        let target = dir.join("a.txt");
        write_str(&target, "hello").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_overwrite_preserves_old_on_crash_simulated() {
        // 无法真正模拟崩溃，但验证正常 overwrite 路径
        let dir = tmp_dir();
        let target = dir.join("b.txt");
        write_str(&target, "v1").unwrap();
        write_str(&target, "v2").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "v2");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_creates_parent_dir() {
        let dir = tmp_dir();
        let target = dir.join("nested").join("deep").join("c.txt");
        write_str(&target, "nested").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "nested");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_no_tmp_left_after_success() {
        let dir = tmp_dir();
        let target = dir.join("d.txt");
        write_str(&target, "clean").unwrap();
        // tmp 文件应已被 persist 消费
        let entries: Vec<_> = fs::read_dir(&dir).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(entries.len(), 1);
        fs::remove_dir_all(dir).ok();
    }
}
