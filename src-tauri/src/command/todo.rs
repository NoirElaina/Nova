// TodoWrite 工具状态查询命令入口。
//
// agent 每次调用 TodoWrite 会替换整列表，前端通过此命令读取最新待办列表，
// 用于在 UI 上实时展示进度。状态本身保存在 shared/todo_state.rs 的全局注册表里。

use tauri::AppHandle;

pub use crate::llm::tools::shared::todo_state::TodoEntry;

/// 读取当前会话的待办列表。会话无待办时返回空数组。
#[tauri::command]
pub async fn list_todos(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Vec<TodoEntry>, String> {
    let _ = app; // 保持与其他命令签名一致，便于后续扩展（如事件推送）。
    Ok(crate::llm::tools::shared::todo_state::global_registry()
        .list(conversation_id.as_deref()))
}
