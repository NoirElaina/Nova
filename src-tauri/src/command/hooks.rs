//! 声明式挂钩配置（hooks.toml）读写命令。

use tauri::AppHandle;

use crate::llm::services::hooks;

/// 读取 hooks.toml 原文；文件不存在返回空串。
#[tauri::command]
pub fn get_hooks_toml(app: AppHandle) -> Result<String, String> {
    let path = hooks::hooks_file_path(&app)?;
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))
}

/// 校验并保存 hooks.toml。解析失败直接返回错误，不落盘。
/// 空内容视为清空（删除文件）。
#[tauri::command]
pub fn save_hooks_toml(app: AppHandle, content: String) -> Result<usize, String> {
    let handler_count = hooks::validate_hooks_toml(&content)?;
    let path = hooks::hooks_file_path(&app)?;

    if content.trim().is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
        }
        return Ok(0);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    crate::llm::utils::atomic_write::write_str(&path, &content)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(handler_count)
}
