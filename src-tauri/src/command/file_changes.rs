use tauri::AppHandle;

pub use crate::llm::services::file_changes::{FileChangeBatch, FileChangeBatchSummary, GitRepoStatus};

#[tauri::command]
pub async fn list_file_changes(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Vec<FileChangeBatchSummary>, String> {
    crate::llm::services::file_changes::list_change_batches(&app, conversation_id.as_deref()).await
}

#[tauri::command]
pub async fn get_file_change(
    app: AppHandle,
    conversation_id: Option<String>,
    batch_id: String,
) -> Result<FileChangeBatch, String> {
    crate::llm::services::file_changes::get_change_batch(
        &app,
        conversation_id.as_deref(),
        &batch_id,
    )
    .await
}

#[tauri::command]
pub async fn revert_file_change(
    app: AppHandle,
    conversation_id: Option<String>,
    batch_id: String,
) -> Result<FileChangeBatch, String> {
    crate::llm::services::file_changes::revert_change_batch(
        &app,
        conversation_id.as_deref(),
        &batch_id,
    )
    .await
}

/// 用户在审查页点击「初始化 Git」按钮时调用。
/// 默认流程不再自动 `git init`，必须由此命令显式触发，避免污染用户工作目录。
/// 返回 (是否新建了 .git, 仓库根绝对路径)。
#[tauri::command]
pub async fn init_git_repo(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<InitGitRepoResult, String> {
    let (created, path) = crate::llm::services::file_changes::init_conversation_repo(
        &app,
        conversation_id.as_deref(),
    )?;
    Ok(InitGitRepoResult { created, path })
}

/// 查询会话工作区的 git 初始化状态，供前端按钮决定文案/可见性。
#[tauri::command]
pub async fn get_git_repo_status(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<GitRepoStatus, String> {
    crate::llm::services::file_changes::get_conversation_repo_status(&app, conversation_id.as_deref())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitGitRepoResult {
    /// true 表示这次调用新建了 `.git`；false 表示仓库已存在，本次为空操作。
    pub created: bool,
    pub path: String,
}
