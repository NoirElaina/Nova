//! 会话取消控制兼容入口：底层状态机已升级至 `session_runtime`。

pub use crate::agent::session_runtime::{
    begin_turn, cancel_all, finish_turn, get_token, is_cancelled, request_cancel,
};
