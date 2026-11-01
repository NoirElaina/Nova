
// Re-export all domain types and functions from the core workspace service
pub use crate::services::workspace::*;

pub fn workspace_list_directory(
    core: &crate::core::Core,
    conversation_id: Option<String>,
    path: Option<String>,
) -> Result<WorkspaceDirectoryListing, String> {
    let root = workspace_root_for_conversation(&core, conversation_id.as_deref())?;
    list_directory_for_root(root, path)
}

pub fn workspace_read_text_file(
    core: &crate::core::Core,
    conversation_id: Option<String>,
    path: String,
) -> Result<WorkspaceFileContent, String> {
    let root = workspace_root_for_conversation(&core, conversation_id.as_deref())?;
    read_text_file(root, path)
}

/// 会话工作区根目录（用户终端等宿主场景需要；返回字符串便于线传输）。
pub fn get_workspace_root(
    core: &crate::core::Core,
    conversation_id: Option<String>,
) -> Result<String, String> {
    workspace_root_for_conversation(&core, conversation_id.as_deref())
        .map(|path| path.display().to_string())
}
