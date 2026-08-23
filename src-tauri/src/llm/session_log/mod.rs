//! 会话事件日志：会话内发生事实的 append-only 记录，唯一事实源。
//!
//! 模型上下文、UI 聊天历史、工具执行日志全部从事件流投影得出，
//! 不再各自维护一份互相手工同步的数据（旧双轨制已废弃）。

pub mod events;
pub mod projection;
pub mod store;

pub use events::SessionEvent;
pub use store::{
    append_event, append_events, delete_events, load_events, update_last_assistant_message_cost,
    StoredEvent,
};
