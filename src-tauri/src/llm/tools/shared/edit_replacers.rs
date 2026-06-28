// EditTool / MultiEditTool 共用的字符串匹配器。
//
// 核心思想：AI 经常写出与文件实际内容差一两个空格/缩进的 old_string，
// 与其直接报错让 AI 重试（消耗一轮 tool call），不如按精度从高到低尝试
// 多种 fuzzy 匹配策略，找到最可能的对应位置。
//
// 移植自 opencode 的 edit.ts，按以下顺序尝试：
// 1. SimpleReplacer — 精确匹配（必须先尝试，保证不破坏严格场景）
// 2. LineTrimmedReplacer — 行内 trim 后匹配
// 3. BlockAnchorReplacer — 首尾行做锚点 + Levenshtein 相似度
// 4. WhitespaceNormalizedReplacer — 空白归一化匹配
// 5. IndentationFlexibleReplacer — 缩进无关匹配
// 6. EscapeNormalizedReplacer — 反转义匹配（AI 把 \n 当换行）
// 7. TrimmedBoundaryReplacer — 边界 trim 匹配
// 8. ContextAwareReplacer — 上下文锚点匹配
//
// 匹配成功后返回实际命中文本（可能与 old_string 不同），调用方用命中
// 文本做真正的替换，保证替换位置正确。

/// 单次匹配产生的候选：实际命中的字符串切片。
/// 一个 replacer 可能产生多个候选（如多处模糊匹配）。
pub struct MatchCandidate {
    /// 实际在 content 中命中的字符串（用于 replace）。
    pub matched_text: String,
    /// 命中起始字节位置。
    pub start: usize,
}

/// 单个 replacer 的 trait。按精度从高到低顺序调用。
pub trait Replacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate>;
}

// ============================================================================
// 1. SimpleReplacer — 精确匹配
// ============================================================================

pub struct SimpleReplacer;

impl Replacer for SimpleReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(idx) = content[start..].find(find) {
            let abs = start + idx;
            out.push(MatchCandidate {
                matched_text: find.to_string(),
                start: abs,
            });
            start = abs + find.len();
        }
        out
    }
}

// ============================================================================
// 2. LineTrimmedReplacer — 行内 trim 后匹配（容错首尾空白）
// ============================================================================

pub struct LineTrimmedReplacer;

impl Replacer for LineTrimmedReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let original_lines: Vec<&str> = content.split('\n').collect();
        let mut search_lines: Vec<&str> = find.split('\n').collect();
        // 去掉末尾空行（常见于 AI 输出尾随换行）
        if search_lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            search_lines.pop();
        }

        let mut out = Vec::new();
        let search_len = search_lines.len();
        if search_len == 0 || original_lines.len() < search_len {
            return out;
        }

        for i in 0..=original_lines.len() - search_len {
            let mut matches = true;
            for j in 0..search_len {
                if original_lines[i + j].trim() != search_lines[j].trim() {
                    matches = false;
                    break;
                }
            }
            if matches {
                // 计算字节起始位置和命中长度
                let mut start_byte = 0;
                for k in 0..i {
                    start_byte += original_lines[k].len() + 1; // +1 for \n
                }
                let mut end_byte = start_byte;
                for k in 0..search_len {
                    end_byte += original_lines[i + k].len();
                    if k < search_len - 1 {
                        end_byte += 1;
                    }
                }
                if end_byte <= content.len() {
                    out.push(MatchCandidate {
                        matched_text: content[start_byte..end_byte].to_string(),
                        start: start_byte,
                    });
                }
            }
        }
        out
    }
}

// ============================================================================
// 3. BlockAnchorReplacer — 首尾行做锚点 + Levenshtein 相似度
// ============================================================================

const SINGLE_CANDIDATE_SIMILARITY_THRESHOLD: f64 = 0.65;
const MULTIPLE_CANDIDATES_SIMILARITY_THRESHOLD: f64 = 0.65;

/// Levenshtein 编辑距离。用于衡量两字符串相似度。
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }
    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr: Vec<usize> = vec![0; b_len + 1];
    for i in 1..=a_len {
        curr[0] = i;
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

pub struct BlockAnchorReplacer;

impl Replacer for BlockAnchorReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let original_lines: Vec<&str> = content.split('\n').collect();
        let mut search_lines: Vec<&str> = find.split('\n').collect();
        if search_lines.len() < 3 {
            return Vec::new();
        }
        if search_lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            search_lines.pop();
        }
        let search_size = search_lines.len();
        if search_size < 3 {
            return Vec::new();
        }

        let first_line = search_lines[0].trim();
        let last_line = search_lines[search_size - 1].trim();
        let max_line_delta = (1).max(search_size / 4);

        // 收集所有首尾锚点匹配的候选块
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        for i in 0..original_lines.len() {
            if original_lines[i].trim() != first_line {
                continue;
            }
            // 找后续第一个匹配尾锚点的行
            for j in (i + 2)..original_lines.len() {
                if original_lines[j].trim() == last_line {
                    let actual_size = j - i + 1;
                    if (actual_size as i64 - search_size as i64).abs() as usize <= max_line_delta {
                        candidates.push((i, j));
                    }
                    break;
                }
            }
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        // 单候选：用松弛阈值
        if candidates.len() == 1 {
            let (start_line, end_line) = candidates[0];
            let actual_size = end_line - start_line + 1;
            let lines_to_check = (search_size - 2).min(actual_size - 2);
            let mut similarity = 0.0;
            if lines_to_check > 0 {
                for j in 1..(search_size - 1).min(actual_size - 1) {
                    let orig_line = original_lines[start_line + j].trim();
                    let search_line = search_lines[j].trim();
                    let max_len = orig_line.len().max(search_line.len());
                    if max_len == 0 {
                        continue;
                    }
                    let dist = levenshtein(orig_line, search_line);
                    similarity += (1.0 - dist as f64 / max_len as f64) / lines_to_check as f64;
                    if similarity >= SINGLE_CANDIDATE_SIMILARITY_THRESHOLD {
                        break;
                    }
                }
            } else {
                similarity = 1.0;
            }
            if similarity >= SINGLE_CANDIDATE_SIMILARITY_THRESHOLD {
                if let Some(c) = build_candidate(content, &original_lines, start_line, end_line) {
                    return vec![c];
                }
            }
            return Vec::new();
        }

        // 多候选：取相似度最高者
        let mut best: Option<(usize, usize)> = None;
        let mut max_sim = -1.0;
        for &(start_line, end_line) in &candidates {
            let actual_size = end_line - start_line + 1;
            let lines_to_check = (search_size - 2).min(actual_size - 2);
            let mut similarity = 0.0;
            if lines_to_check > 0 {
                for j in 1..(search_size - 1).min(actual_size - 1) {
                    let orig_line = original_lines[start_line + j].trim();
                    let search_line = search_lines[j].trim();
                    let max_len = orig_line.len().max(search_line.len());
                    if max_len == 0 {
                        continue;
                    }
                    let dist = levenshtein(orig_line, search_line);
                    similarity += 1.0 - dist as f64 / max_len as f64;
                }
                similarity /= lines_to_check as f64;
            } else {
                similarity = 1.0;
            }
            if similarity > max_sim {
                max_sim = similarity;
                best = Some((start_line, end_line));
            }
        }

        if max_sim >= MULTIPLE_CANDIDATES_SIMILARITY_THRESHOLD {
            if let Some((start_line, end_line)) = best {
                if let Some(c) = build_candidate(content, &original_lines, start_line, end_line) {
                    return vec![c];
                }
            }
        }
        Vec::new()
    }
}

/// 从行索引范围构建 MatchCandidate。
fn build_candidate(
    content: &str,
    lines: &[&str],
    start_line: usize,
    end_line: usize,
) -> Option<MatchCandidate> {
    let mut start_byte = 0;
    for k in 0..start_line {
        start_byte += lines[k].len() + 1;
    }
    let mut end_byte = start_byte;
    for k in start_line..=end_line {
        end_byte += lines[k].len();
        if k < end_line {
            end_byte += 1;
        }
    }
    if end_byte > content.len() {
        return None;
    }
    Some(MatchCandidate {
        matched_text: content[start_byte..end_byte].to_string(),
        start: start_byte,
    })
}

// ============================================================================
// 4. WhitespaceNormalizedReplacer — 空白归一化匹配
// ============================================================================

pub struct WhitespaceNormalizedReplacer;

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Replacer for WhitespaceNormalizedReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let normalized_find = normalize_whitespace(find);
        if normalized_find.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let lines: Vec<&str> = content.split('\n').collect();

        // 单行匹配
        for (i, line) in lines.iter().enumerate() {
            if normalize_whitespace(line) == normalized_find {
                let mut start_byte = 0;
                for k in 0..i {
                    start_byte += lines[k].len() + 1;
                }
                out.push(MatchCandidate {
                    matched_text: line.to_string(),
                    start: start_byte,
                });
            }
        }

        // 多行匹配
        let find_lines: Vec<&str> = find.split('\n').collect();
        if find_lines.len() > 1 {
            for i in 0..=lines.len().saturating_sub(find_lines.len()) {
                let block: String = lines[i..i + find_lines.len()].join("\n");
                if normalize_whitespace(&block) == normalized_find {
                    let mut start_byte = 0;
                    for k in 0..i {
                        start_byte += lines[k].len() + 1;
                    }
                    out.push(MatchCandidate {
                        matched_text: block,
                        start: start_byte,
                    });
                }
            }
        }
        out
    }
}

// ============================================================================
// 5. IndentationFlexibleReplacer — 缩进无关匹配
// ============================================================================

pub struct IndentationFlexibleReplacer;

fn remove_indentation(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let non_empty: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return text.to_string();
    }
    let min_indent = non_empty
        .iter()
        .map(|l| {
            let match_len = l.chars().take_while(|c| c.is_whitespace()).count();
            match_len
        })
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| if l.trim().is_empty() { *l } else { &l[min_indent.min(l.len())..] })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Replacer for IndentationFlexibleReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let normalized_find = remove_indentation(find);
        let content_lines: Vec<&str> = content.split('\n').collect();
        let find_lines: Vec<&str> = find.split('\n').collect();
        let mut out = Vec::new();
        if find_lines.is_empty() || content_lines.len() < find_lines.len() {
            return out;
        }
        for i in 0..=content_lines.len() - find_lines.len() {
            let block: String = content_lines[i..i + find_lines.len()].join("\n");
            if remove_indentation(&block) == normalized_find {
                let mut start_byte = 0;
                for k in 0..i {
                    start_byte += content_lines[k].len() + 1;
                }
                out.push(MatchCandidate {
                    matched_text: block,
                    start: start_byte,
                });
            }
        }
        out
    }
}

// ============================================================================
// 6. EscapeNormalizedReplacer — 反转义匹配
// ============================================================================

pub struct EscapeNormalizedReplacer;

fn unescape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                let unescaped = match next {
                    'n' => Some('\n'),
                    't' => Some('\t'),
                    'r' => Some('\r'),
                    '\'' => Some('\''),
                    '"' => Some('"'),
                    '`' => Some('`'),
                    '\\' => Some('\\'),
                    '$' => Some('$'),
                    _ => None,
                };
                if let Some(u) = unescaped {
                    out.push(u);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

impl Replacer for EscapeNormalizedReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let unescaped_find = unescape_string(find);
        if unescaped_find.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();

        // 直接匹配反转义后的字符串
        if let Some(idx) = content.find(&unescaped_find) {
            out.push(MatchCandidate {
                matched_text: unescaped_find.clone(),
                start: idx,
            });
        }

        // 块匹配：content 中的块反转义后等于 find 反转义
        let lines: Vec<&str> = content.split('\n').collect();
        let find_lines: Vec<&str> = unescaped_find.split('\n').collect();
        if find_lines.len() > 1 && lines.len() >= find_lines.len() {
            for i in 0..=lines.len() - find_lines.len() {
                let block: String = lines[i..i + find_lines.len()].join("\n");
                if unescape_string(&block) == unescaped_find {
                    let mut start_byte = 0;
                    for k in 0..i {
                        start_byte += lines[k].len() + 1;
                    }
                    out.push(MatchCandidate {
                        matched_text: block,
                        start: start_byte,
                    });
                }
            }
        }
        out
    }
}

// ============================================================================
// 7. TrimmedBoundaryReplacer — 边界 trim 匹配
// ============================================================================

pub struct TrimmedBoundaryReplacer;

impl Replacer for TrimmedBoundaryReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let trimmed_find = find.trim();
        if trimmed_find.is_empty() || trimmed_find == find {
            return Vec::new();
        }
        let mut out = Vec::new();

        // 直接 trim 匹配
        if let Some(idx) = content.find(trimmed_find) {
            out.push(MatchCandidate {
                matched_text: trimmed_find.to_string(),
                start: idx,
            });
        }

        // 块 trim 匹配
        let lines: Vec<&str> = content.split('\n').collect();
        let find_lines: Vec<&str> = find.split('\n').collect();
        if find_lines.len() > 1 && lines.len() >= find_lines.len() {
            for i in 0..=lines.len() - find_lines.len() {
                let block: String = lines[i..i + find_lines.len()].join("\n");
                if block.trim() == trimmed_find {
                    let mut start_byte = 0;
                    for k in 0..i {
                        start_byte += lines[k].len() + 1;
                    }
                    out.push(MatchCandidate {
                        matched_text: block,
                        start: start_byte,
                    });
                }
            }
        }
        out
    }
}

// ============================================================================
// 8. ContextAwareReplacer — 上下文锚点匹配
// ============================================================================

pub struct ContextAwareReplacer;

impl Replacer for ContextAwareReplacer {
    fn find(&self, content: &str, find: &str) -> Vec<MatchCandidate> {
        let mut find_lines: Vec<&str> = find.split('\n').collect();
        if find_lines.len() < 3 {
            return Vec::new();
        }
        if find_lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            find_lines.pop();
        }
        if find_lines.len() < 3 {
            return Vec::new();
        }
        let content_lines: Vec<&str> = content.split('\n').collect();
        let first_line = find_lines[0].trim();
        let last_line = find_lines[find_lines.len() - 1].trim();

        let mut out = Vec::new();
        for i in 0..content_lines.len() {
            if content_lines[i].trim() != first_line {
                continue;
            }
            for j in (i + 2)..content_lines.len() {
                if content_lines[j].trim() == last_line {
                    let block_lines = &content_lines[i..=j];
                    if block_lines.len() == find_lines.len() {
                        // 中间行至少 50% 匹配
                        let mut matching = 0;
                        let mut total_non_empty = 0;
                        for k in 1..block_lines.len() - 1 {
                            let b = block_lines[k].trim();
                            let f = find_lines[k].trim();
                            if !b.is_empty() || !f.is_empty() {
                                total_non_empty += 1;
                                if b == f {
                                    matching += 1;
                                }
                            }
                        }
                        if total_non_empty == 0
                            || matching as f64 / total_non_empty as f64 >= 0.5
                        {
                            let block: String = block_lines.join("\n");
                            let mut start_byte = 0;
                            for k in 0..i {
                                start_byte += content_lines[k].len() + 1;
                            }
                            out.push(MatchCandidate {
                                matched_text: block,
                                start: start_byte,
                            });
                            break; // 只取首个
                        }
                    }
                    break;
                }
            }
        }
        out
    }
}

// ============================================================================
// 主入口：按精度从高到低尝试所有 replacer
// ============================================================================

/// 匹配结果。调用方根据 variant 决定如何处理。
pub enum FindResult {
    /// 唯一匹配（命中位置 + 实际命中文本）。
    Unique { matched_text: String, start: usize },
    /// 多个匹配（拒绝替换，提示用户提供更多上下文）。
    Multiple(usize),
    /// 完全没找到。
    NotFound,
}

/// 检查匹配是否"不成比例"——命中文本比 old_string 长太多，
/// 可能是误匹配到更大的块。
fn is_disproportionate_match(search: &str, old_string: &str) -> bool {
    let old_lines = old_string.split('\n').count();
    let search_lines = search.split('\n').count();
    if search_lines >= old_lines + 3 && search_lines >= old_lines * 2 {
        return true;
    }
    if old_lines == 1 {
        return false;
    }
    let search_trim_len = search.trim().len();
    let old_trim_len = old_string.trim().len();
    search_trim_len > old_trim_len + 500 && search_trim_len > old_trim_len * 4
}

/// 按 replacer 链查找。返回唯一匹配 / 多匹配 / 未找到。
/// `replace_all=true` 时多匹配也视为成功（调用方自己处理全部替换）。
pub fn find_match(content: &str, old_string: &str, replace_all: bool) -> FindResult {
    let replacers: Vec<Box<dyn Replacer>> = vec![
        Box::new(SimpleReplacer),
        Box::new(LineTrimmedReplacer),
        Box::new(BlockAnchorReplacer),
        Box::new(WhitespaceNormalizedReplacer),
        Box::new(IndentationFlexibleReplacer),
        Box::new(EscapeNormalizedReplacer),
        Box::new(TrimmedBoundaryReplacer),
        Box::new(ContextAwareReplacer),
    ];

    let mut not_found = true;
    for replacer in &replacers {
        let candidates = replacer.find(content, old_string);
        if candidates.is_empty() {
            continue;
        }
        not_found = false;

        // 检查每个候选是否"不成比例"
        for c in &candidates {
            if is_disproportionate_match(&c.matched_text, old_string) {
                // 拒绝这次匹配，继续尝试下一个 replacer
                continue;
            }
        }

        if replace_all {
            // 全部替换模式：用第一个候选（精确匹配的话会有多个）
            // 但只有 SimpleReplacer 会返回多个精确匹配，其他 replacer 通常返回 1 个
            if let Some(first) = candidates.first() {
                return FindResult::Unique {
                    matched_text: first.matched_text.clone(),
                    start: first.start,
                };
            }
        }

        // 非 replace_all：需要唯一匹配
        // 去重：不同 replacer 可能产生相同候选，按 start 去重
        let mut unique_starts: Vec<&MatchCandidate> = Vec::new();
        for c in &candidates {
            if !unique_starts.iter().any(|u| u.start == c.start) {
                unique_starts.push(c);
            }
        }
        if unique_starts.is_empty() {
            continue;
        }
        if unique_starts.len() == 1 {
            let c = unique_starts[0];
            if is_disproportionate_match(&c.matched_text, old_string) {
                continue;
            }
            return FindResult::Unique {
                matched_text: c.matched_text.clone(),
                start: c.start,
            };
        }
        // 多匹配：返回数量让调用方提示用户
        return FindResult::Multiple(unique_starts.len());
    }

    if not_found {
        FindResult::NotFound
    } else {
        // 所有候选都被 disproportionate 检查拒绝
        FindResult::NotFound
    }
}

/// 执行替换。返回 (新内容, 替换次数)。
/// 调用方需先调用 find_match 确认能匹配。
pub fn apply_replace(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    match find_match(content, old_string, replace_all) {
        FindResult::Unique { matched_text, .. } => {
            if replace_all {
                let count = content.matches(&matched_text).count();
                Ok((content.replace(&matched_text, new_string), count))
            } else {
                Ok((content.replacen(&matched_text, new_string, 1), 1))
            }
        }
        FindResult::Multiple(n) => Err(format!(
            "old_string found {} times in file (not unique). \
             Use replace_all: true to replace all occurrences, \
             or provide more context to make old_string unique.",
            n
        )),
        FindResult::NotFound => Err(format!(
            "old_string not found in file.\n\n\
             Tip: Read the file again to get exact content. \
             Common causes: mismatched whitespace (tabs vs spaces), \
             trailing whitespace, line ending differences (CRLF vs LF), \
             or the text was modified since last read."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_replacer_exact() {
        let result = apply_replace("hello world", "hello", "hi", false).unwrap();
        assert_eq!(result.0, "hi world");
        assert_eq!(result.1, 1);
    }

    #[test]
    fn test_simple_replacer_multiple() {
        let result = apply_replace("foo bar foo", "foo", "x", false);
        assert!(matches!(result, Err(_)));
    }

    #[test]
    fn test_simple_replacer_replace_all() {
        let result = apply_replace("foo bar foo", "foo", "x", true).unwrap();
        assert_eq!(result.0, "x bar x");
        assert_eq!(result.1, 2);
    }

    #[test]
    fn test_line_trimmed_replacer() {
        // AI 写了 "  hello  " 但文件里是 "  hello"
        let content = "  hello\nworld";
        let find = "  hello  ";
        let result = apply_replace(content, find, "hi", false).unwrap();
        assert_eq!(result.0, "hi\nworld");
    }

    #[test]
    fn test_whitespace_normalized_replacer() {
        // AI 写了 "hello   world" 但文件里是 "hello world"
        let content = "hello world";
        let find = "hello   world";
        let result = apply_replace(content, find, "hi", false).unwrap();
        assert_eq!(result.0, "hi");
    }

    #[test]
    fn test_not_found() {
        let result = apply_replace("hello world", "nonexistent", "x", false);
        assert!(result.is_err());
    }
}
