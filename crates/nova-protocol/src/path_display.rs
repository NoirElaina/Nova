//! 路径显示归一化（纯函数）：剥离 Windows canonicalize 引入的 \\?\ verbatim 前缀。
//! 该形式回显进工具输出与系统提示词，普通形式对文件系统操作同样有效。

use std::path::Path;

pub fn display_path_text(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
        const VERBATIM_PREFIX: &str = r"\\?\";

        if let Some(rest) = path.strip_prefix(VERBATIM_UNC_PREFIX) {
            return format!(r"\\{}", rest);
        }
        if let Some(rest) = path.strip_prefix(VERBATIM_PREFIX) {
            return rest.to_string();
        }
    }

    path.to_string()
}

pub fn display_path_string(path: &Path) -> String {
    display_path_text(&path.display().to_string())
}
