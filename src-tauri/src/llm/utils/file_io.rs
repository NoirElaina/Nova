// Write/Edit 工具共用的文件 I/O 辅助。
//
// 读取时把文件解码为 UTF-8 String、剥离 BOM、CRLF→LF，
// 让模型永远只看到 LF 内容（模型几乎总是输出 \n 的 old_string）；
// 同时记录原始编码与行尾（FileMeta），写回时按需还原。
//
// 行尾策略：
//   - WriteTool：模型写什么落什么，不还原行尾（write_text_content_lf）。
//     保留旧 CRLF 会在覆盖 CRLF 文件时把 bash 脚本写入 \r，损坏脚本。
//   - EditTool/MultiEditTool：按原始文件 endings 还原（write_file_with_meta）
//
// 原子写入：所有落盘走 atomic_write 模块（tempfile + fsync + rename + 权限保留 + symlink 解析）。
// 这避免了 Windows 上 fs::write 截断/损坏文件的风险。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::llm::utils::fingerprint::FileFingerprint;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

// ─────────────────────────────────────────────
// 全局文件内容缓存（对标 claude-code fileReadCache 的指纹缓存模式）。
// 同一轮内 Read→Edit→再 Read 的重复读盘全部命中内存；
// 写盘后失效对应条目（写失效不写更新，避免编码探测歧义）。
// ─────────────────────────────────────────────

/// 条目上限（与 claude-code fileReadCache 对齐）。
const CONTENT_CACHE_MAX_ENTRIES: usize = 1000;
/// 总字节上限（对标 fileStateCache 量级），插入超限时从最旧逐条驱逐。
const CONTENT_CACHE_MAX_BYTES: usize = 25 * 1024 * 1024;

#[derive(Default)]
struct FileContentCache {
    /// 插入序（FIFO 驱逐）。
    order: VecDeque<PathBuf>,
    entries: HashMap<PathBuf, (FileFingerprint, String, FileMeta)>,
    total_bytes: usize,
}

fn content_cache() -> &'static Mutex<FileContentCache> {
    static CACHE: std::sync::OnceLock<Mutex<FileContentCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(FileContentCache::default()))
}

/// 规范化缓存 key：尽量用 canonicalize 消除 `/`与`\`、`..`、大小写差异（对齐 read_state 的 path_key）。
fn content_cache_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 写盘后失效缓存条目（供写入路径与测试使用）。
pub(crate) fn invalidate_file_content_cache(path: &Path) {
    let key = content_cache_key(path);
    let mut cache = content_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, content, _)) = cache.entries.remove(&key) {
        cache.total_bytes = cache.total_bytes.saturating_sub(content.len());
        cache.order.retain(|p| p != &key);
    }
}

/// 驱逐超限条目：从最旧逐条移除，直到条目数与总字节都回到阈值内。
fn evict_if_needed(cache: &mut FileContentCache) {
    while cache.entries.len() > CONTENT_CACHE_MAX_ENTRIES
        || cache.total_bytes > CONTENT_CACHE_MAX_BYTES
    {
        let Some(oldest) = cache.order.pop_front() else {
            break;
        };
        if let Some((_, content, _)) = cache.entries.remove(&oldest) {
            cache.total_bytes = cache.total_bytes.saturating_sub(content.len());
        }
    }
}

/// 文件原始编码。读取时探测，写回时据此还原（含 BOM）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileEncoding {
    /// 无 BOM 的 UTF-8（最常见）。
    Utf8,
    /// 带 BOM 的 UTF-8（常见于 Windows 记事本保存的文件）。
    Utf8Bom,
    /// 带 BOM 的 UTF-16 小端（Windows 部分工具默认）。
    Utf16Le,
    /// 带 BOM 的 UTF-16 大端。
    Utf16Be,
}

/// 文件原始行尾风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEnding {
    /// `\n`
    Lf,
    /// `\r\n`
    Crlf,
}

/// 文件元信息：写回时用于还原编码与行尾。
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileMeta {
    pub encoding: FileEncoding,
    pub line_ending: LineEnding,
}

/// 读取文件并归一化：解码为 UTF-8 String、剥离 BOM、CRLF→LF。
///
/// 返回 `(归一化内容, 文件元信息)`。归一化内容是模型应当看到的内容
/// （纯 LF、无 BOM）；元信息供 `write_file_with_meta` 写回时还原。
/// 指纹未变时命中全局内容缓存，避免同一轮内重复读盘。
pub(crate) fn read_file_meta(path: &Path) -> Result<(String, FileMeta), String> {
    let fingerprint = FileFingerprint::of(path);
    let key = content_cache_key(path);
    // 指纹命中：直接返回缓存克隆。
    if let Some(fp) = fingerprint {
        let cache = content_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_fp, content, meta)) = cache.entries.get(&key) {
            if *cached_fp == fp {
                return Ok((content.clone(), *meta));
            }
        }
    }

    let bytes =
        std::fs::read(path).map_err(|e| format!("Error reading {}: {}", path.display(), e))?;
    let (decoded, encoding) = decode_bytes(&bytes, path)?;

    // 探测行尾：只要出现一次 CRLF 即视为 CRLF 文件。
    let line_ending = if decoded.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };

    // 归一化为 LF，让模型只看到 \n。
    let normalized = match line_ending {
        LineEnding::Crlf => decoded.replace("\r\n", "\n"),
        LineEnding::Lf => decoded,
    };

    let meta = FileMeta { encoding, line_ending };
    // 入缓存：指纹不可得（罕见，刚读完元数据就失效）时跳过，不影响正确性。
    if let Some(fp) = fingerprint {
        let mut cache = content_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, old_content, _)) = cache.entries.remove(&key) {
            cache.total_bytes = cache.total_bytes.saturating_sub(old_content.len());
            cache.order.retain(|p| p != &key);
        }
        cache.total_bytes += normalized.len();
        cache.order.push_back(key.clone());
        cache.entries.insert(key, (fp, normalized.clone(), meta));
        evict_if_needed(&mut cache);
    }

    Ok((normalized, meta))
}

/// 按元信息写回：把 LF 内容还原为原始行尾，并按原始编码（含 BOM）编码后原子写盘。
///
/// 用于 EditTool / MultiEditTool：保留原文件的行尾与编码。
/// 落盘走 atomic_write（tempfile + rename + 权限保留 + symlink 解析）。
pub(crate) fn write_file_with_meta(
    target: &Path,
    content_lf: &str,
    meta: &FileMeta,
) -> Result<String, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", target.display(), e))?;
    }

    // 还原行尾。content_lf 已归一为纯 LF，直接整体替换不会产生 \r\r\n。
    let restored = match meta.line_ending {
        LineEnding::Lf => content_lf.to_string(),
        LineEnding::Crlf => content_lf.replace('\n', "\r\n"),
    };

    let bytes = encode_bytes(&restored, meta.encoding);
    crate::llm::utils::atomic_write::write_bytes(target, &bytes)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
    invalidate_file_content_cache(target);
    Ok(target.display().to_string())
}

/// 不还原行尾的直写：模型 content 里是什么就落什么，仅按 encoding 编码后原子写盘。
///
/// 用于 WriteTool。模型在 content 里发送的就是它要的行尾，写什么落什么，不还原旧文件行尾。
/// 保留旧 CRLF 会在覆盖 CRLF 文件时把 bash 脚本写入 \r，损坏脚本。
pub(crate) fn write_text_content_lf(
    target: &Path,
    content: &str,
    encoding: FileEncoding,
) -> Result<String, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", target.display(), e))?;
    }

    let bytes = encode_bytes(content, encoding);
    crate::llm::utils::atomic_write::write_bytes(target, &bytes)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
    invalidate_file_content_cache(target);
    Ok(target.display().to_string())
}

/// 根据字节首部探测编码并解码为 String（已剥离 BOM）。
fn decode_bytes(bytes: &[u8], path: &Path) -> Result<(String, FileEncoding), String> {
    // UTF-16LE BOM: FF FE
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let s = decode_utf16(&bytes[2..], true, path)?;
        return Ok((s, FileEncoding::Utf16Le));
    }
    // UTF-16BE BOM: FE FF
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let s = decode_utf16(&bytes[2..], false, path)?;
        return Ok((s, FileEncoding::Utf16Be));
    }
    // UTF-8 BOM: EF BB BF
    if bytes.len() >= 3 && &bytes[0..3] == UTF8_BOM {
        let s = String::from_utf8(bytes[3..].to_vec())
            .map_err(|e| format!("File {} is not valid UTF-8: {}", path.display(), e))?;
        return Ok((s, FileEncoding::Utf8Bom));
    }
    // 默认 UTF-8 无 BOM。
    let s = String::from_utf8(bytes.to_vec())
        .map_err(|e| format!("File {} is not valid UTF-8: {}", path.display(), e))?;
    Ok((s, FileEncoding::Utf8))
}

/// 把 UTF-16 字节体（不含 BOM）解码为 String。
fn decode_utf16(body: &[u8], little_endian: bool, path: &Path) -> Result<String, String> {
    if body.len() % 2 != 0 {
        return Err(format!(
            "File {} has an odd-length UTF-16 body and cannot be decoded.",
            path.display()
        ));
    }
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16(&units)
        .map_err(|e| format!("File {} is not valid UTF-16: {}", path.display(), e))
}

/// 按编码把内容编码为字节（含 BOM）。
fn encode_bytes(content: &str, encoding: FileEncoding) -> Vec<u8> {
    match encoding {
        FileEncoding::Utf8 => content.as_bytes().to_vec(),
        FileEncoding::Utf8Bom => {
            let mut v = Vec::with_capacity(UTF8_BOM.len() + content.len());
            v.extend_from_slice(UTF8_BOM);
            v.extend_from_slice(content.as_bytes());
            v
        }
        FileEncoding::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            for u in content.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        }
        FileEncoding::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            for u in content.encode_utf16() {
                v.extend_from_slice(&u.to_be_bytes());
            }
            v
        }
    }
}

pub fn resolve_tool_path(raw_path: &str) -> Result<std::path::PathBuf, String> {
    crate::llm::utils::paths::resolve_absolute_path_for_write(raw_path, "path")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "__nova_fio_{}_{}__.txt",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn second_read_hits_cache_and_write_invalidates() {
        let path = unique_temp_path("rw");
        std::fs::write(&path, "line1\r\nline2\r\n").expect("write");

        let (first, meta) = read_file_meta(&path).expect("first read");
        assert_eq!(first, "line1\nline2\n");
        assert_eq!(meta.line_ending, LineEnding::Crlf);

        // 第二次读应命中缓存（内容一致）。
        let (second, _) = read_file_meta(&path).expect("second read");
        assert_eq!(first, second);

        // 绕过缓存直改磁盘内容（长度变化 → 指纹变），再读必须看到新内容。
        std::fs::write(&path, "changed\n").expect("external rewrite");
        let (third, meta3) = read_file_meta(&path).expect("third read");
        assert_eq!(third, "changed\n");
        assert_eq!(meta3.line_ending, LineEnding::Lf);

        // 写入路径失效缓存：写后重读拿到新内容。
        write_text_content_lf(&path, "after write", FileEncoding::Utf8).expect("write lf");
        let (fourth, _) = read_file_meta(&path).expect("fourth read");
        assert_eq!(fourth, "after write");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn invalidate_removes_entry() {
        let path = unique_temp_path("inv");
        std::fs::write(&path, "abc").expect("write");
        read_file_meta(&path).expect("read");
        let key = content_cache_key(&path);
        assert!(content_cache()
            .lock()
            .unwrap()
            .entries
            .contains_key(&key));
        invalidate_file_content_cache(&path);
        assert!(!content_cache()
            .lock()
            .unwrap()
            .entries
            .contains_key(&key));
        let _ = std::fs::remove_file(&path);
    }
}
