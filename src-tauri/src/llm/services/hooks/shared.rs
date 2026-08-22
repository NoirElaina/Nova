use crate::llm::types::{Content, ContentBlock, Message, Role};

// 取最近一条 assistant 消息的纯文本（Stop 事件的 stopWhen 匹配与 payload 用）。
pub(crate) fn latest_assistant_text(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|m| {
            if m.role != Role::Assistant {
                return None;
            }
            match &m.content {
                Content::Text(t) => Some(t.clone()),
                Content::Blocks(blocks) => Some(
                    blocks
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            }
        })
        .unwrap_or_default()
}

// 历史中是否已存在完全相同的 user 文本消息（Stop 上下文注入去重用）。
pub(crate) fn has_exact_user_message(messages: &[Message], expected: &str) -> bool {
    messages.iter().any(|m| {
        if m.role != Role::User {
            return false;
        }
        matches!(&m.content, Content::Text(text) if text == expected)
    })
}
