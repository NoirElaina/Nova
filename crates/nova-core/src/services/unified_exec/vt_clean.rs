//! 终端 PTY/管道流清理与 UTF-8 增量解码工具。
//!
//! 职责：
//! 1. `Utf8StreamDecoder`: 跨 chunk 边界安全解码 UTF-8，绝不在多字节中文字符中间切碎。
//! 2. `StreamingVtSanitizer`: 状态机式剥离 ANSI/CSI/OSC/DCS/ConPTY 控制序列，
//!    同时保留 ConPTY 隐含换行（`CUP` 行号递增）与行内光标前移空白（`CUF`），避免信息粘连。
//! 3. `collapse_line_controls`: 处理 `\r\n` -> `\n`、单 `\r` 进度条覆盖与 `\x08` 退格。
//! 4. `truncate_output_tokens`: 对超限输出执行头尾保留 + UTF-8 字符边界安全截断。

/// 增量 UTF-8 解码器：把跨 `read()` 边界切开的半个多字节序列暂存到下一批字节到达。
#[derive(Debug, Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub fn push_bytes(&mut self, bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return String::new();
        }
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();

        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    out.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(err) => {
                    let valid_up_to = err.valid_up_to();
                    if valid_up_to > 0 {
                        // Safety: `valid_up_to` is guaranteed by `str::from_utf8` to be valid UTF-8.
                        let prefix = std::str::from_utf8(&self.pending[..valid_up_to])
                            .unwrap_or_default();
                        out.push_str(prefix);
                    }
                    match err.error_len() {
                        Some(invalid_len) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..valid_up_to + invalid_len);
                        }
                        None => {
                            if valid_up_to > 0 {
                                self.pending.drain(..valid_up_to);
                            }
                            break;
                        }
                    }
                }
            }
        }

        out
    }

    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let flushed = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        flushed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Default)]
enum EscState {
    #[default]
    Ground,
    EscStart,
    Csi { buf: String },
    Osc { buf: String, prev_esc: bool },
    StringSeq { prev_esc: bool, len: usize },
    CharsetConsume,
}


const MAX_ESC_SEQ_LEN: usize = 512;

/// 流式终端控制序列净化器（线程独占状态，无锁）。
#[derive(Debug, Default)]
pub struct StreamingVtSanitizer {
    state: EscState,
    last_cup_row: Option<u32>,
    line_has_visible: bool,
    pending_cr: bool,
    pending_replies: Vec<&'static [u8]>,
}

impl StreamingVtSanitizer {
    /// 取出流式解析期间识别到的终端查询（如 ConPTY 必发的 `\x1b[6n` DSR 光标查询）对应的应答字节。
    pub fn take_terminal_replies(&mut self) -> Vec<&'static [u8]> {
        std::mem::take(&mut self.pending_replies)
    }

    pub fn push_str(&mut self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());

        for ch in input.chars() {
            match &mut self.state {
                EscState::Ground => {
                    if self.pending_cr {
                        if ch == '\n' {
                            out.push('\n');
                            self.pending_cr = false;
                            self.line_has_visible = false;
                            continue;
                        } else if ch != '\r' {
                            out.push('\r');
                            self.pending_cr = false;
                        }
                    }

                    match ch {
                        '\x1b' => {
                            self.state = EscState::EscStart;
                        }
                        '\r' => {
                            self.pending_cr = true;
                        }
                        '\n' => {
                            out.push('\n');
                            self.line_has_visible = false;
                        }
                        '\t' => {
                            out.push('\t');
                            self.line_has_visible = true;
                        }
                        '\x08' => {
                            out.push('\x08');
                        }
                        c if c.is_control() => {
                            // 跳过 BEL(\x07), NUL(\x00) 等非排版控制符
                        }
                        c => {
                            out.push(c);
                            self.line_has_visible = true;
                        }
                    }
                }
                EscState::EscStart => match ch {
                    '[' => {
                        self.state = EscState::Csi { buf: String::new() };
                    }
                    ']' => {
                        self.state = EscState::Osc {
                            buf: String::new(),
                            prev_esc: false,
                        };
                    }
                    'P' | 'X' | '^' | '_' => {
                        self.state = EscState::StringSeq {
                            prev_esc: false,
                            len: 0,
                        };
                    }
                    '(' | ')' | '*' | '+' | '-' | '.' | '/' | '#' | ' ' => {
                        self.state = EscState::CharsetConsume;
                    }
                    // 合法的两字符 ESC 序列（DECSC/DECKPAM/IND/RI/NEL…）整体丢弃。
                    '7' | '8' | '=' | '>' | 'D' | 'E' | 'H' | 'M' | 'N' | 'O' | 'V' | 'W'
                    | 'Z' | '\\' | 'c' | 'n' | 'o' => {
                        self.state = EscState::Ground;
                    }
                    // 孤立 ESC（二进制数据、被截断的序列）：ESC 丢弃，
                    // 但它后面的字符是正常内容，必须保留，否则会静默吞字。
                    other => {
                        self.state = EscState::Ground;
                        out.push(other);
                    }
                },
                EscState::CharsetConsume => {
                    self.state = EscState::Ground;
                }
                EscState::Csi { buf } => {
                    if ('@'..='~').contains(&ch) {
                        let csi_body = std::mem::take(buf);
                        self.state = EscState::Ground;
                        self.apply_csi(&csi_body, ch, &mut out);
                    } else if buf.len() >= MAX_ESC_SEQ_LEN {
                        self.state = EscState::Ground;
                    } else {
                        buf.push(ch);
                    }
                }
                EscState::Osc { buf, prev_esc } => {
                    if ch == '\x07' {
                        let osc_body = std::mem::take(buf);
                        self.state = EscState::Ground;
                        self.handle_osc_query(&osc_body);
                    } else if *prev_esc && ch == '\\' {
                        let mut osc_body = std::mem::take(buf);
                        if osc_body.ends_with('\x1b') {
                            osc_body.pop();
                        }
                        self.state = EscState::Ground;
                        self.handle_osc_query(&osc_body);
                    } else if buf.len() >= MAX_ESC_SEQ_LEN {
                        self.state = EscState::Ground;
                    } else {
                        *prev_esc = ch == '\x1b';
                        buf.push(ch);
                    }
                }
                EscState::StringSeq { prev_esc, len } => {
                    *len += 1;
                    if ch == '\x07' || (*prev_esc && ch == '\\') || *len >= MAX_ESC_SEQ_LEN {
                        self.state = EscState::Ground;
                    } else {
                        *prev_esc = ch == '\x1b';
                    }
                }
            }
        }

        out
    }

    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if self.pending_cr {
            self.pending_cr = false;
        }
        self.state = EscState::Ground;
        out.shrink_to_fit();
        out
    }

    fn handle_osc_query(&mut self, body: &str) {
        if body == "10;?" {
            self.pending_replies.push(b"\x1b]10;rgb:e5e5/e5e5/e5e5\x07");
        } else if body == "11;?" {
            self.pending_replies.push(b"\x1b]11;rgb:1212/1313/1616\x07");
        }
    }

    fn apply_csi(&mut self, body: &str, final_ch: char, out: &mut String) {
        // 识别终端状态/光标/设备属性查询，供无头 PTY 线程即时应答，防止 ConPTY 等进程永久阻塞等待终端回包
        match (body, final_ch) {
            ("6", 'n') => {
                self.pending_replies.push(b"\x1b[1;1R");
                return;
            }
            ("?6", 'n') => {
                self.pending_replies.push(b"\x1b[?1;1R");
                return;
            }
            ("5", 'n') => {
                self.pending_replies.push(b"\x1b[0n");
                return;
            }
            ("" | "0", 'c') => {
                self.pending_replies.push(b"\x1b[?1;2c");
                return;
            }
            (">" | ">0", 'c') => {
                self.pending_replies.push(b"\x1b[>0;0;1c");
                return;
            }
            _ => {}
        }

        // 私有模式序列（?9001h / ?1004h / ?25l / >0c 等）直接丢弃
        if body.starts_with('?') || body.starts_with('>') || body.starts_with('<') || body.starts_with('=') {
            return;
        }

        match final_ch {
            // CUP / HVP: ESC [ row ; col H|f
            // Windows ConPTY 在多行铺屏时可能直接用行号递增定位而不发 \r\n
            'H' | 'f' => {
                let mut parts = body.split(';');
                let row = parts
                    .next()
                    .and_then(|s| if s.is_empty() { None } else { s.parse::<u32>().ok() })
                    .unwrap_or(1);
                let prev = self.last_cup_row;
                self.last_cup_row = Some(row);
                if self.line_has_visible {
                    let moved_down = prev.map(|p| row > p).unwrap_or(row > 1);
                    if moved_down {
                        out.push('\n');
                        self.line_has_visible = false;
                    }
                }
            }
            // CUF: Cursor Forward ESC [ n C (ConPTY 有时将多空格压缩为 CUF)
            'C' => {
                if self.line_has_visible {
                    let count = body.parse::<usize>().unwrap_or(1).clamp(1, 16);
                    for _ in 0..count {
                        out.push(' ');
                    }
                }
            }
            // CUD: Cursor Down ESC [ n B
            'B' => {
                if self.line_has_visible {
                    let count = body.parse::<usize>().unwrap_or(1).clamp(1, 4);
                    for _ in 0..count {
                        out.push('\n');
                    }
                    self.line_has_visible = false;
                }
            }
            // ED: Erase in Display — 若行尚未写完可视内容则清屏噪声忽略
            'J' => {
                let mode = body.parse::<u32>().unwrap_or(0);
                if mode == 2 || mode == 3 {
                    self.last_cup_row = None;
                }
            }
            // EL: Erase in Line ESC [ K / 0K -> \x0b (erase col..end), 2K -> \x0c (clear line)
            'K' => {
                let mode = body.parse::<u32>().unwrap_or(0);
                if mode == 2 {
                    out.push('\x0c');
                } else if mode == 0 {
                    out.push('\x0b');
                }
            }
            _ => {}
        }
    }
}

/// 折叠单个 chunk 内的 `\r` 行覆盖、`\x1b[K` 行擦除与 `\x08` 退格，规范为干净的 `\n` 文本。
pub fn collapse_line_controls(text: &str) -> String {
    if !text.contains('\r')
        && !text.contains('\x08')
        && !text.contains('\x0b')
        && !text.contains('\x0c')
    {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut current: Vec<char> = Vec::new();
    let mut col: usize = 0;

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    for c in current.drain(..) {
                        result.push(c);
                    }
                    result.push('\n');
                    col = 0;
                } else {
                    col = 0;
                }
            }
            '\n' => {
                for c in current.drain(..) {
                    result.push(c);
                }
                result.push('\n');
                col = 0;
            }
            '\x08' => {
                col = col.saturating_sub(1);
            }
            '\x0b' => {
                if col < current.len() {
                    current.truncate(col);
                }
            }
            '\x0c' => {
                current.clear();
                col = 0;
            }
            other => {
                if col < current.len() {
                    current[col] = other;
                } else {
                    while current.len() < col {
                        current.push(' ');
                    }
                    current.push(other);
                }
                col += 1;
            }
        }
    }

    for c in current {
        result.push(c);
    }
    result
}

/// 一次性清理原始终端输出（用于单元测试及静态文本归一化）。
pub fn clean_terminal_output(raw: &str) -> String {
    let mut sanitizer = StreamingVtSanitizer::default();
    let mut cleaned = sanitizer.push_str(raw);
    cleaned.push_str(&sanitizer.finish());
    collapse_line_controls(&cleaned)
}

#[derive(Debug, Clone)]
pub struct TruncatedOutput {
    pub text: String,
    pub original_bytes: usize,
    pub approx_original_tokens: usize,
    pub truncated: bool,
}

/// 按 token 预算执行头尾保留截断（对齐 Codex unified_exec：每 token 约 4 字节）。
pub fn truncate_output_tokens(text: &str, max_output_tokens: usize) -> TruncatedOutput {
    let original_bytes = text.len();
    let approx_original_tokens = original_bytes.div_ceil(4);
    let clamped_tokens = max_output_tokens.clamp(64, 100_000);
    let max_bytes = clamped_tokens.saturating_mul(4);

    if original_bytes <= max_bytes {
        return TruncatedOutput {
            text: text.to_string(),
            original_bytes,
            approx_original_tokens,
            truncated: false,
        };
    }

    // 40% 头部 + 60% 尾部：编译报错/服务失败日志主要集中在尾部
    let head_budget = (max_bytes * 2) / 5;
    let tail_budget = max_bytes.saturating_sub(head_budget);

    let mut head_cut = head_budget.min(original_bytes);
    while head_cut > 0 && !text.is_char_boundary(head_cut) {
        head_cut -= 1;
    }

    let mut tail_start = original_bytes.saturating_sub(tail_budget);
    while tail_start < original_bytes && !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    if tail_start < head_cut {
        tail_start = head_cut;
    }

    let omitted_bytes = tail_start.saturating_sub(head_cut);
    let omitted_tokens = omitted_bytes.div_ceil(4);

    let formatted = format!(
        "{}\n\n...{} tokens truncated...\n\n{}",
        &text[..head_cut],
        omitted_tokens,
        &text[tail_start..]
    );

    TruncatedOutput {
        text: formatted,
        original_bytes,
        approx_original_tokens,
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_decoder_preserves_split_multibyte_chinese() {
        let sample = "你好，世界🚀测试".as_bytes();
        let mut decoder = Utf8StreamDecoder::default();
        let mut out = String::new();
        // 按 1 字节和 2 字节极端切分
        for chunk in sample.chunks(2) {
            out.push_str(&decoder.push_bytes(chunk));
        }
        out.push_str(&decoder.finish());
        assert_eq!(out, "你好，世界🚀测试");
    }

    #[test]
    fn cleans_conpty_and_ansi_noise() {
        let raw = "\x1b[?9001h\x1b[?1004h\x1b]0;PowerShell 7\x07\x1b[?25l\x1b[2J\x1b[m\x1b[H\x1b[32mhello\x1b[0m\x1b[2Cworld\r\nline2\r\n";
        let cleaned = clean_terminal_output(raw);
        assert_eq!(cleaned, "hello  world\nline2\n");
    }

    #[test]
    fn streaming_handles_split_escape_across_chunks() {
        let mut sanitizer = StreamingVtSanitizer::default();
        let part1 = sanitizer.push_str("abc\x1b[38;2;");
        let part2 = sanitizer.push_str("255;0;0mdef\x1b]0;WinTitle");
        let part3 = sanitizer.push_str("\x07!\r\n");
        let combined = format!("{}{}{}", part1, part2, part3);
        assert_eq!(collapse_line_controls(&combined), "abcdef!\n");
    }

    #[test]
    fn lone_escape_does_not_eat_following_char() {
        // 二进制数据或被截断的序列里常见的孤立 ESC：
        // 只丢弃 ESC 本身，后面的正常字符必须保留。
        let cleaned = clean_terminal_output("abc\u{1b}def");
        assert_eq!(cleaned, "abcdef");
        // 合法的两字符 ESC 序列仍然整体丢弃。
        let saved_cursor = clean_terminal_output("a\u{1b}7b");
        assert_eq!(saved_cursor, "ab");
    }

    #[test]
    fn carriage_return_and_erase_line_cleans_progress() {
        let cleaned = clean_terminal_output("loading 100%\r\x1b[Kdone!\r\nnext");
        assert_eq!(cleaned, "done!\nnext");
    }

    #[test]
    fn token_truncation_keeps_utf8_boundaries() {
        let text = "中文前缀".repeat(120) + &"尾部报错信息".repeat(120);
        let res = truncate_output_tokens(&text, 100);
        assert!(res.truncated);
        assert!(res.text.contains("tokens truncated"));
        assert!(res.text.starts_with("中文前缀"));
        assert!(res.text.ends_with("尾部报错信息"));
    }
}
