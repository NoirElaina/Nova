// Write/Edit 工具共用的文件 I/O 辅助。
// 这些 helper 与 git 审查无关，按职责归入 utils。
//
// 设计对齐 Claude Code 的 FileEditTool：
//   读取时把文件解码为 UTF-8 String、剥离 BOM、并把 CRLF 归一成 LF，
//   让模型永远只看到 LF 内容（模型几乎总是输出 \n 的 old_string）；
//   同时记录原始编码与行尾（FileMeta），写回时据此还原，保持文件原貌。
// 这避免了 Windows 上 CRLF 文件因行尾不匹配导致 Edit 失败或写出混合行尾。

use std::path::Path;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF8_BOM_STR: &str = "\u{FEFF}";

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
pub(crate) fn read_file_meta(path: &Path) -> Result<(String, FileMeta), String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("Error reading {}: {}", path.display(), e))?;
    let (decoded, encoding) = decode_bytes(&bytes, path)?;

    // 探测行尾：只要出现一次 CRLF 即视为 CRLF 文件（对齐 Claude Code）。
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

    Ok((normalized, FileMeta { encoding, line_ending }))
}

/// 按元信息写回：把 LF 内容还原为原始行尾，并按原始编码（含 BOM）编码后写盘。
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
    std::fs::write(target, bytes)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
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

/// 读取文件为归一化 UTF-8 文本（剥离 BOM、CRLF→LF），返回 `(内容, 是否带 UTF-8 BOM)`。
///
/// 供 ReadTool 使用：让模型看到干净的 LF 内容，避免 copy 出来的 old_string
/// 因 BOM / CRLF 与磁盘文件不一致导致匹配失败。
pub(crate) fn read_file_utf8(path: &Path) -> Result<(String, bool), String> {
    let (content, meta) = read_file_meta(path)?;
    Ok((content, meta.encoding == FileEncoding::Utf8Bom))
}

pub fn resolve_tool_path(raw_path: &str) -> Result<std::path::PathBuf, String> {
    crate::llm::utils::paths::resolve_absolute_path_for_write(raw_path, "path")
}

/// Write/Edit 工具专用：写文件并返回受影响的 *display* 路径。
///
/// 不保留 BOM、不做行尾转换——仅供 WriteTool 覆盖/创建整文件使用
/// （模型提供的整文件内容按 UTF-8 / LF 写入）。EditTool / MultiEditTool
/// 应改用 `write_file_with_meta` 以保留原文件的编码与行尾。
pub fn write_file_simple(target: &Path, content: &str) -> Result<String, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", target.display(), e))?;
    }
    // 防御：剥掉模型可能误带的 BOM 前缀。
    let content = content.strip_prefix(UTF8_BOM_STR).unwrap_or(content);
    std::fs::write(target, content)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
    Ok(target.display().to_string())
}
