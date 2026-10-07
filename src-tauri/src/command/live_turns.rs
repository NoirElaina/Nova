//! 实时轮次状态查询与确认命令入口。

use crate::agent::lifecycle::live_turns::{self, LiveTurnStatus};

#[tauri::command]
pub fn get_chat_turn_status(
    conversation_id: Option<String>,
) -> Result<Option<LiveTurnStatus>, String> {
    Ok(live_turns::get_status(conversation_id.as_deref()))
}

#[tauri::command]
pub fn ack_chat_turn_status(conversation_id: Option<String>) -> Result<bool, String> {
    Ok(live_turns::ack_status(conversation_id.as_deref()))
}
