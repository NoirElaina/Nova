// 所有 provider 共用的 SSE 解析工具函数。
// 这里的函数与具体协议无关，只处理 SSE 帧边界和数据提取。

/// 将字符串截断到最多 `max_chars` 个字符，超出时末尾附加 `...`。
pub(super) fn truncate_for_log(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", preview)
    } else {
        preview
    }
}

/// 在字节切片中搜索 needle 的第一次出现位置。
pub(super) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 在 SSE 缓冲区中寻找最近的事件分隔符（`\n\n` 或 `\r\n\r\n`）。
/// 返回 `(事件结束字节位置, 分隔符字节长度)`。
pub(super) fn find_sse_event_delimiter(input: &[u8]) -> Option<(usize, usize)> {
    let lf = find_bytes(input, b"\n\n").map(|idx| (idx, 2));
    let crlf = find_bytes(input, b"\r\n\r\n").map(|idx| (idx, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

/// 从原始 SSE 事件文本中提取 `data:` 行的内容，多行数据用 `\n` 拼接。
pub(super) fn extract_sse_data(event_raw: &str) -> String {
    event_raw
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix("data:")
                .map(|data| data.trim_start().to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}
