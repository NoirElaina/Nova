// Write/Edit 工具共用的文件 I/O 辅助。
// 这些 helper 与 git 审查无关，按职责归入 utils。

use std::path::Path;

/// Read file as UTF-8 string, stripping BOM (\u{FEFF}) if present.
pub(crate) fn read_file_utf8(path: &Path) -> Result<String, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Error reading {}: {}", path.display(), e))?;
    Ok(content
        .strip_prefix('\u{FEFF}')
        .unwrap_or(&content)
        .to_string())
}

pub fn resolve_tool_path(raw_path: &str) -> Result<std::path::PathBuf, String> {
    crate::llm::utils::paths::resolve_absolute_path_for_write(raw_path, "path")
}

/// Write/Edit 工具专用：堆文件并返回受影响的 *display* 路径。
pub fn write_file_simple(target: &Path, content: &str) -> Result<String, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", target.display(), e))?;
    }
    std::fs::write(target, content)
        .map_err(|e| format!("写入文件失败 {}: {}", target.display(), e))?;
    Ok(target.display().to_string())
}
