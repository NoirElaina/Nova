use crate::services::shell_sessions::{self, BackgroundOutput, ShellSessionStatus};

pub async fn get_shell_session_status(
    core: &crate::core::Core,
    conversation_id: Option<String>,
) -> Result<ShellSessionStatus, String> {
    let mut status = shell_sessions::session_status(conversation_id.as_deref()).await;
    if status.cwd.is_none() {
        status.cwd = Some(
            crate::api::workspace::workspace_root_string_for_conversation(
                &core,
                conversation_id.as_deref(),
            )?,
        );
    }
    Ok(status)
}

/// 列出会话的后台作业（不含输出内容），供输入框上方的"后台任务"小框与右侧面板使用。
pub async fn list_background_jobs(
    conversation_id: Option<String>,
) -> Result<Vec<BackgroundOutput>, String> {
    Ok(shell_sessions::list_background_jobs_for_conversation(
        conversation_id.as_deref(),
    ))
}

/// 读取单个后台作业的状态与最近输出。id 全局唯一；conversation_id 只用于错误提示。
pub async fn read_background_job_output(
    conversation_id: Option<String>,
    id: String,
    tail_lines: Option<usize>,
) -> Result<BackgroundOutput, String> {
    let _ = &conversation_id;
    shell_sessions::read_background_output(id.trim(), None, tail_lines)
}
