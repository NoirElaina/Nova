//! 会话标题派生工具。

/// 从首条用户消息派生会话标题：取首行、按字符截断到 24 字符。
pub fn derive_title_from_message(content: &str) -> String {
    // 取首行文本作为标题候选。
    let first_line = content.lines().next().unwrap_or("").trim();
    // 首行为空时回退到全文裁剪。
    let source = if first_line.is_empty() {
        content.trim()
    } else {
        first_line
    };
    // 标题最大字符数。
    let max_chars = 24usize;
    // 构建截断后的标题。
    let mut out = String::new();
    // 逐字符截断，避免 UTF-8 字节切分问题。
    for ch in source.chars().take(max_chars) {
        out.push(ch);
    }
    // 原文超过上限时追加省略号。
    if source.chars().count() > max_chars {
        format!("{}...", out)
    } else if out.is_empty() {
        // 为空时给默认标题。
        "New chat".to_string()
    } else {
        // 返回截断后的标题。
        out
    }
}
