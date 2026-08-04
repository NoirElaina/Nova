// 会话 plan 查询命令入口。
//
// plan 文件由 exit_plan_mode 工具写入 {app_data_dir}/plans/{conversation_id}.md，
// 每会话仅一份。前端加载会话时通过此命令读取，用于渲染结构化 Plan 面板。

use tauri::AppHandle;

use crate::llm::services::plan_files::{self, ConversationPlan};
use crate::llm::utils::error_event::report_backend_result;

/// 读取当前会话的 plan；会话没有 plan 时返回 null。
#[tauri::command]
pub async fn get_conversation_plan(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Option<ConversationPlan>, String> {
    let result = plan_files::load_conversation_plan(&app, conversation_id.as_deref());
    report_backend_result(&app, "command.plan.get_conversation_plan", result, None)
}
