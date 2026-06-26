use tauri::AppHandle;

pub use crate::llm::services::git_ops::{GitRepoStatus, WorkspaceDiff};

/// 收集会话工作区的 git diff（已跟踪 + untracked），供审查页实时展示。
#[tauri::command]
pub async fn get_workspace_diff(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<WorkspaceDiff, String> {
    crate::llm::services::git_ops::collect_conversation_diff(&app, conversation_id.as_deref())
}

/// 查询会话工作区的 git 初始化状态，供前端按钮决定文案/可见性。
#[tauri::command]
pub async fn get_git_repo_status(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<GitRepoStatus, String> {
    crate::llm::services::git_ops::get_conversation_repo_status(&app, conversation_id.as_deref())
}

/// 基于显式工作区路径查询 git 状态。供 EnvironmentBar 在无会话时使用。
#[tauri::command]
pub async fn get_workspace_git_status(
    workspace_path: String,
) -> Result<GitRepoStatus, String> {
    crate::llm::services::git_ops::get_repo_status_by_path(&workspace_path)
}

/// 用户在审查页点击「初始化 Git」按钮时调用。
/// 返回 (是否新建了 .git, 仓库根绝对路径)。
#[tauri::command]
pub async fn init_git_repo(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<InitGitRepoResult, String> {
    let (created, path) = crate::llm::services::git_ops::init_conversation_repo(
        &app,
        conversation_id.as_deref(),
    )?;
    Ok(InitGitRepoResult { created, path })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitGitRepoResult {
    /// true 表示这次调用新建了 `.git`；false 表示仓库已存在，本次为空操作。
    pub created: bool,
    pub path: String,
}
