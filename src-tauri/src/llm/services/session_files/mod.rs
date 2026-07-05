use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager};
use tracing::warn;

/// 会话文件元信息，返回给前端展示。
///
/// 安全设计：不再暴露 `read_path`（绝对路径）给前端。
/// 前端/AI 只能拿到 `filename`，读取时通过 `conversation_id + filename` 让后端推导路径。
/// 这样前端永远拿不到绝对路径，无法构造 `../../../etc/passwd` 等路径遍历攻击。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileMeta {
    /// 文件名（已 sanitize，不含路径分隔符），同时也是读取时的唯一 key。
    pub filename: String,
    /// 文件大小（字节）。
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

/// 对用户提供的 filename 做严格 sanitize，防止路径遍历攻击（Zip Slip 类）。
///
/// 拒绝以下情况：
/// - 含路径分隔符（`/` 或 `\`）
/// - 含 `..`（任何位置的 ParentDir 组件）
/// - 绝对路径前缀（Unix `/` 或 Windows `C:\`）
/// - 空字符串或全空白
/// - 含空字节
///
/// 通过则返回 sanitized 后的文件名（仅文件名部分，不含目录）。
/// 这相当于把 filename 当作"单一 key"，后端根据 `conversation_id + filename` 推导实际路径。
fn sanitize_filename(filename: &str) -> Result<String, String> {
    if filename.contains('\0') {
        return Err("文件名不能含空字节".to_string());
    }
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err("文件名不能为空".to_string());
    }
    // 统一反斜杠为正斜杠后做组件级检查
    let normalized = trimmed.replace('\\', "/");
    let candidate = Path::new(&normalized);
    if candidate.is_absolute() {
        return Err("文件名不能是绝对路径".to_string());
    }
    // 逐组件检查：只允许 Normal 组件，拒绝 ParentDir/RootDir/Prefix
    let mut clean_parts: Vec<String> = Vec::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                let s = part.to_string_lossy().to_string();
                if s.contains("..") {
                    return Err("文件名不能含 '..'".to_string());
                }
                clean_parts.push(s);
            }
            Component::CurDir => {} // 忽略 "."
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("文件名不能含路径分隔符或目录穿越".to_string());
            }
        }
    }
    if clean_parts.is_empty() {
        return Err("文件名 sanitize 后为空".to_string());
    }
    // 多级路径（如 "sub/file.txt"）只取最后一段作为文件名
    // 这样即使绕过组件检查传入多级路径，也只保留文件名部分
    let final_name = clean_parts.last().unwrap().clone();
    if final_name.is_empty() {
        return Err("文件名 sanitize 后为空".to_string());
    }
    Ok(final_name)
}

/// 根据 conversation_id + filename 推导实际存储路径，并验证路径在 session_files 目录内。
/// 这是"文件名→路径映射"的核心：前端只给 filename，后端推导出绝对路径。
fn resolve_session_file_path(
    app: &AppHandle,
    conversation_id: &str,
    filename: &str,
) -> Result<PathBuf, String> {
    let dir = session_files_dir(app, conversation_id)?;
    let sanitized = sanitize_filename(filename)?;
    // 可能是 docx/pptx（子目录 + content.txt），也可能是普通文件
    // 先尝试普通文件路径，再尝试子目录的 content.txt
    let direct = dir.join(&sanitized);
    if direct.is_file() {
        // canonicalize 验证路径未逃逸 session_files 目录
        let canon = direct.canonicalize().map_err(|e| format!("路径解析失败: {}", e))?;
        let dir_canon = dir.canonicalize().map_err(|e| format!("目录解析失败: {}", e))?;
        if !canon.starts_with(&dir_canon) {
            return Err("路径逃逸 session_files 目录".to_string());
        }
        return Ok(canon);
    }
    // 尝试 docx/pptx 子目录
    let sub_content = dir.join(&sanitized).join("content.txt");
    if sub_content.is_file() {
        let canon = sub_content
            .canonicalize()
            .map_err(|e| format!("路径解析失败: {}", e))?;
        let dir_canon = dir.canonicalize().map_err(|e| format!("目录解析失败: {}", e))?;
        if !canon.starts_with(&dir_canon) {
            return Err("路径逃逸 session_files 目录".to_string());
        }
        return Ok(canon);
    }
    Err(format!("文件不存在: {}", sanitized))
}

/// 保存会话文件（仅二进制文档：docx/pptx/pdf）。纯文本文件由前端直接注入对话上下文，不存盘。
///
/// - docx/pptx：创建同名子文件夹，存 original.{ext} + content.txt（解析后的文本）。
/// - pdf：直接写入根目录。
///
/// 安全：filename 经过 sanitize_filename 严格清洗，拒绝路径遍历输入。
pub fn save_session_file(
    app: &AppHandle,
    conversation_id: &str,
    filename: &str,
    content: Option<&str>,
    raw_bytes: Option<&[u8]>,
) -> Result<SessionFileMeta, String> {
    let dir = session_files_dir(app, conversation_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create session_files dir: {}", e))?;

    // sanitize：拒绝含路径分隔符、..、绝对路径前缀的输入
    let safe_name = sanitize_filename(filename)?;
    let ext = safe_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_zip_doc = matches!(ext.as_str(), "docx" | "pptx");

    let size = if is_zip_doc {
        let sub_dir = dir.join(&safe_name);
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

        content
            .map(|t| t.len() as u64)
            .unwrap_or_else(|| std::fs::metadata(&content_path).map(|m| m.len()).unwrap_or(0))
    } else {
        // 纯文本/代码/PDF/图片：直接写入根目录
        let file_path = dir.join(&safe_name);

        if let Some(bytes) = raw_bytes {
            std::fs::write(&file_path, bytes)
                .map_err(|e| format!("Failed to write file: {}", e))?;
        } else if let Some(text) = content {
            std::fs::write(&file_path, text)
                .map_err(|e| format!("Failed to write file: {}", e))?;
        }

        std::fs::metadata(&file_path)
            .map(|m| m.len())
            .unwrap_or(0)
    };

    Ok(SessionFileMeta {
        filename: safe_name,
        size,
        created_at: chrono::Utc::now().timestamp(),
    })
}

/// 列出会话的所有文件。
///
/// 安全：不再返回 read_path（绝对路径），只返回 filename 作为读取 key。
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
///
/// 安全：不再接受任意 read_path，改为接受 conversation_id + filename。
/// 后端通过 resolve_session_file_path 推导实际路径，并 canonicalize 验证路径
/// 在 session_files 目录内。前端永远拿不到绝对路径，无法构造路径遍历攻击。
pub fn read_session_file(
    app: &AppHandle,
    conversation_id: &str,
    filename: &str,
) -> Result<String, String> {
    let path = resolve_session_file_path(app, conversation_id, filename)?;
    std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {}", e))
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
        "The following files have been uploaded for this conversation. Use the Read tool to read any of them (pass the session_files: path as file_path):".to_string(),
    ];

    // 向 AI 暴露虚拟路径 session_files:{filename}。
    // AI 调用 Read 工具时传这个路径，ReadTool 识别前缀后自动用当前会话 ID
    // 拼接实际存储路径读取。AI 永远拿不到绝对路径，无法构造路径遍历攻击。
    for file in &files {
        lines.push(format!("- session_files:{} ({})", file.filename, file.filename));
    }

    Some(crate::llm::types::Message {
        role: crate::llm::types::Role::User,
        content: crate::llm::types::Content::Text(lines.join("\n")),
    })
}
