// Write/Edit 工具共用的文件 I/O 辅助。
// 这些 helper 与 git 审查无关，按职责归入 utils。

use std::path::Path;

const UTF8_BOM: &str = "\u{FEFF}";

/// Read file as UTF-8 string, returning (content_without_bom, had_bom)。
///
/// BOM 在读取时剥离，让 AI 看到干净内容（避免 AI copy 出来的 old_string
/// 带 BOM 但磁盘文件不带 BOM 导致匹配失败）；had_bom 标记在写回时用于
/// 恢复 BOM，保持文件原貌。
pub(crate) fn read_file_utf8(path: &Path) -> Result<(String, bool), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Error reading {}: {}", path.display(), e))?;
    if let Some(stripped) = content.strip_prefix(UTF8_BOM) {
        Ok((stripped.to_string(), true))
    } else {
        Ok((content, false))
    }
}

pub fn resolve_tool_path(raw_path: &str) -> Result<std::path::PathBuf, String> {
    crate::llm::utils::paths::resolve_absolute_path_for_write(raw_path, "path")
}

/// Write file, restoring UTF-8 BOM if `had_bom` is true。
///
/// EditTool 用这个保证编辑前后 BOM 状态一致——有 BOM 的 Windows 文件
/// 编辑后仍然有 BOM，无 BOM 的文件不会被意外加上 BOM。
pub fn write_file_preserving(
    target: &Path,
    content: &str,
    had_bom: bool,
) -> Result<String, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", target.display(), e))?;
    }
    let final_content = if had_bom {
        format!("{UTF8_BOM}{content}")
    } else {
        content.to_string()
    };
    std::fs::write(target, final_content)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
    Ok(target.display().to_string())
}

/// Write/Edit 工具专用：堆文件并返回受影响的 *display* 路径。
///
/// 不保留 BOM——仅供 WriteTool 覆盖整文件使用。EditTool 应改用
/// `write_file_preserving`。
pub fn write_file_simple(target: &Path, content: &str) -> Result<String, String> {
    write_file_preserving(target, content, false)
}
