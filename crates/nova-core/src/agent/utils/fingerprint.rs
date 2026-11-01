//! 统一文件指纹：mtime + 文件大小。
//!
//! 全项目所有"文件内容缓存"共用这一指纹类型（设置 / 文件内容 / 系统提示词 /
//! hooks.toml），避免各处重复实现判重逻辑。
//!
//! 为什么用 mtime + size 双条件：Windows 的 mtime 精度有限，同秒内的等长覆写
//! 可能不改变 mtime；叠加文件大小变化可以兼容绝大多数编辑场景（两者都未变则
//! 视为未变）。不追求密码学级别的变更检测——真实来源变更都有对应的
//! invalidate 调用兜底。

use std::path::Path;
use std::time::SystemTime;

/// 文件指纹：修改时间 + 字节大小。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFingerprint {
    pub mtime: SystemTime,
    pub size: u64,
}

impl FileFingerprint {
    /// 读取文件指纹；文件不存在或元数据读取失败返回 None。
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta.modified().ok()?;
        Some(Self {
            mtime,
            size: meta.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_of_missing_file_is_none() {
        let path = std::env::temp_dir().join("__nova_fp_missing_12345__.txt");
        assert!(FileFingerprint::of(&path).is_none());
    }

    #[test]
    fn fingerprint_changes_after_rewrite() {
        let path = std::env::temp_dir().join("__nova_fp_rewrite_12345__.txt");
        std::fs::write(&path, "hello").expect("write");
        let first = FileFingerprint::of(&path).expect("fingerprint");
        // 改变长度必然改变指纹（同秒覆写场景靠 size 兜底）。
        std::fs::write(&path, "hello world").expect("rewrite");
        let second = FileFingerprint::of(&path).expect("fingerprint after rewrite");
        assert_ne!(first, second);
        let _ = std::fs::remove_file(&path);
    }
}
