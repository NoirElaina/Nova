use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tracing::warn;

/// 会话文件元信息，返回给前端展示。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileMeta {
    /// 文件名（不含路径），同时也是子文件夹名。
    pub filename: String,
    /// AI 读取用的完整绝对路径（content.txt 或 original.ext）。
    pub read_path: String,
    /// 原始文件大小（字节）。
    pub size: u64,
    /// 创建时间（Unix 秒）。
    pub created_at: i64,
}

/// 会话文件存储根目录：{app_data_dir}/session_files/{conversation_id}/
fn session_files_dir(app: &AppHandle, conversation_id: &str) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app_data_dir: {}", e))?;
    Ok(data_dir.join("session_files").join(conversation_id))
}

/// 保存会话文件（仅二进制文档：docx/pptx/pdf）。纯文本文件由前端直接注入对话上下文，不存盘。
///
/// - docx/pptx：创建同名子文件夹，存 original.{ext} + content.txt（解析后的文本）。
/// - pdf：直接写入根目录。
pub fn save_session_file(
    app: &AppHandle,
    conversation_id: &str,
    filename: &str,
    content: Option<&str>,
    raw_bytes: Option<&[u8]>,
) -> Result<SessionFileMeta, String> {
    let dir = session_files_dir(app, conversation_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create session_files dir: {}", e))?;

    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_zip_doc = matches!(ext.as_str(), "docx" | "pptx");

    let (read_path, size) = if is_zip_doc {
        let sub_dir = dir.join(filename);
        std::fs::create_dir_all(&sub_dir)
            .map_err(|e| format!("Failed to create file sub-dir: {}", e))?;

        if let Some(bytes) = raw_bytes {
            let original_name = format!("original.{}", ext);
            std::fs::write(sub_dir.join(&original_name), bytes)
                .map_err(|e| format!("Failed to write original file: {}", e))?;
        }

        let content_path = sub_dir.join("content.txt");
        if let Some(text) = content {
            std::fs::write(&content_path, text)
                .map_err(|e| format!("Failed to write content.txt: {}", e))?;
        }

        let size = content
            .map(|t| t.len() as u64)
            .unwrap_or_else(|| std::fs::metadata(&content_path).map(|m| m.len()).unwrap_or(0));
        (content_path.to_string_lossy().to_string(), size)
    } else {
        // 纯文本/代码/PDF/图片：直接写入根目录
        let file_path = dir.join(filename);

        if let Some(bytes) = raw_bytes {
            std::fs::write(&file_path, bytes)
                .map_err(|e| format!("Failed to write file: {}", e))?;
        } else if let Some(text) = content {
            std::fs::write(&file_path, text)
                .map_err(|e| format!("Failed to write file: {}", e))?;
        }

        let size = std::fs::metadata(&file_path)
            .map(|m| m.len())
            .unwrap_or(0);
        (file_path.to_string_lossy().to_string(), size)
    };

    Ok(SessionFileMeta {
        filename: filename.to_string(),
        read_path,
        size,
        created_at: chrono::Utc::now().timestamp(),
    })
}

/// 列出会话的所有文件。
pub fn list_session_files(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<SessionFileMeta>, String> {
    let dir = session_files_dir(app, conversation_id)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();

    for entry in std::fs::read_dir(&dir).map_err(|e| format!("Failed to read dir: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            // docx/pptx 子文件夹
            let content_path = path.join("content.txt");
            if content_path.exists() {
                let size = std::fs::metadata(&content_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                let created_at = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.created().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                files.push(SessionFileMeta {
                    filename: name,
                    read_path: content_path.to_string_lossy().to_string(),
                    size,
                    created_at,
                });
            }
        } else {
            // 根目录下的普通文件
            let size = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(0);
            let created_at = entry
                .metadata()
                .ok()
                .and_then(|m| m.created().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or_else(|| chrono::Utc::now().timestamp());
            files.push(SessionFileMeta {
                filename: name,
                read_path: path.to_string_lossy().to_string(),
                size,
                created_at,
            });
        }
    }

    files.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(files)
}

/// 删除会话的所有文件。
pub fn delete_all_session_files(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let dir = session_files_dir(app, conversation_id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("Failed to remove session files: {}", e))?;
    }
    Ok(())
}

/// 读取会话文件文本内容（供前端 FilesTab 展示）。
pub fn read_session_file(read_path: &str) -> Result<String, String> {
    let path = std::path::Path::new(read_path);
    if !path.is_file() {
        return Err("文件不存在或不是文件".to_string());
    }
    std::fs::read_to_string(path).map_err(|e| format!("读取文件失败: {}", e))
}

/// 删除所有会话的文件（清空历史时调用）。
pub fn delete_all_session_files_all(app: &AppHandle) -> Result<(), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app_data_dir: {}", e))?;
    let root = data_dir.join("session_files");
    if root.exists() {
        std::fs::remove_dir_all(&root)
            .map_err(|e| format!("Failed to remove all session files: {}", e))?;
    }
    Ok(())
}

/// 为 context_assembler 构建会话文件列表文本注入。
pub async fn build_session_files_message(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Option<crate::llm::types::Message> {
    let Some(conv_id) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return None;
    };

    let files = match list_session_files(app, conv_id) {
        Ok(f) => f,
        Err(e) => {
            warn!(error = %e, "Failed to list session files for context injection");
            return None;
        }
    };

    if files.is_empty() {
        return None;
    }

    let mut lines = vec![
        "[Session Files]".to_string(),
        "The following files have been uploaded for this conversation. Use the Read tool to read any of them:".to_string(),
    ];

    for file in &files {
        lines.push(format!("- {} ({})", file.read_path, file.filename));
    }

    Some(crate::llm::types::Message {
        role: crate::llm::types::Role::User,
        content: crate::llm::types::Content::Text(lines.join("\n")),
    })
}
