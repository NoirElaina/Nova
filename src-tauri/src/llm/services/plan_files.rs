// 会话计划（plan）文件存储。
//
// 每个会话只保留一份最新 plan：exit_plan_mode 工具执行时把完整计划写入
// {app_data_dir}/plans/{conversation_id}.md，覆盖旧文件。
// 与 session_files 不同，plan 永远存在应用数据目录，不进用户工作区。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 会话 plan 元信息，返回给前端展示。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPlan {
    /// plan 的 Markdown 全文。
    pub content: String,
    /// plan 文件更新时间（Unix 秒）。
    pub updated_at: i64,
}

/// plan 存储目录：{app_data_dir}/plans/
fn plans_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法读取应用数据目录: {}", e))?
        .join("plans");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 plans 目录失败: {}", e))?;
    Ok(dir)
}

/// conversation_id 来自内部生成（uuid 或 "__default__"），仍做一次保守清洗，
/// 防止未来调用方传入含路径分隔符的值造成目录穿越。
fn safe_conversation_id(conversation_id: Option<&str>) -> String {
    let id = conversation_id.map(|s| s.trim()).unwrap_or("");
    if id.is_empty() {
        return "__default__".to_string();
    }
    id.replace(['/', '\\', '\0'], "_")
}

fn plan_file_path(app: &AppHandle, conversation_id: Option<&str>) -> Result<PathBuf, String> {
    Ok(plans_dir(app)?.join(format!("{}.md", safe_conversation_id(conversation_id))))
}

/// 写入（覆盖）会话 plan 文件，返回更新后的 plan 信息。
pub fn save_conversation_plan(
    app: &AppHandle,
    conversation_id: Option<&str>,
    content: &str,
) -> Result<ConversationPlan, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("plan 内容不能为空".to_string());
    }
    let path = plan_file_path(app, conversation_id)?;
    std::fs::write(&path, trimmed).map_err(|e| format!("写入 plan 文件失败: {}", e))?;
    let updated_at = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Ok(ConversationPlan {
        content: trimmed.to_string(),
        updated_at,
    })
}

/// 读取会话 plan 文件；不存在时返回 Ok(None)。
pub fn load_conversation_plan(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<Option<ConversationPlan>, String> {
    let path = plan_file_path(app, conversation_id)?;
    let content = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("读取 plan 文件失败: {}", err)),
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    let updated_at = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Ok(Some(ConversationPlan {
        content,
        updated_at,
    }))
}

/// 删除会话 plan 文件（删除会话时调用）；文件不存在也视为成功。
pub fn delete_conversation_plan(app: &AppHandle, conversation_id: Option<&str>) -> Result<(), String> {
    let path = plan_file_path(app, conversation_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("删除 plan 文件失败: {}", err)),
    }
}
