//! 具备三级渐进容错与 Unicode 标点归一化的行序列寻址算法。

/// 尝试在 `lines` 中从 `start` 开始定位 `pattern` 序列。
/// 寻址策略按严格程度递减：
/// 1. 精确逐行匹配（Exact Match）；
/// 2. 忽略行尾空白匹配（Rstrip Match）；
/// 3. 忽略首尾空白匹配（Trim Both Match）；
/// 4. Unicode 印刷标点（弯引号、破折号、不换行空格等）归一化匹配。
///
/// 当 `eof` 为 true 时，优先从文件末尾向后搜寻，匹配失败再回退至 `start`。
pub(crate) fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let search_start = if eof && lines.len() >= pattern.len() {
        let eof_start = lines.len() - pattern.len();
        eof_start.max(start)
    } else {
        start
    };

    // 1. 精确匹配
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }

    // 2. 忽略行尾空白
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if lines[i + p_idx].trim_end() != pat.trim_end() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }

    // 3. 忽略首尾空白（容忍模型缩进层级偏移）
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if lines[i + p_idx].trim() != pat.trim() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }

    // 4. Unicode 标点归一化（将弯引号、连字符、破折号、全角空格等转为 ASCII）
    fn normalise(s: &str) -> String {
        s.trim()
            .chars()
            .map(|c| match c {
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
                | '\u{2212}' => '-',
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
                '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
                | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
                | '\u{3000}' => ' ',
                other => other,
            })
            .collect::<String>()
    }

    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if normalise(&lines[i + p_idx]) != normalise(pat) {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }

    // 5. 若 eof 从末尾查找失败，回退尝试从 start 全局正向重试
    if eof && search_start > start {
        return seek_sequence(lines, pattern, start, false);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_vec(strings: &[&str]) -> Vec<String> {
        strings.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_exact_and_fuzzy_matches() {
        let file = to_vec(&["fn foo() {", "    let a = 1;", "    println!(\"hi\");", "}"]);
        // 精确匹配
        assert_eq!(seek_sequence(&file, &to_vec(&["    let a = 1;"]), 0, false), Some(1));
        // 缩进容错匹配
        assert_eq!(seek_sequence(&file, &to_vec(&["let a = 1;"]), 0, false), Some(1));
        // 行尾空格容错
        assert_eq!(seek_sequence(&file, &to_vec(&["    let a = 1;   "]), 0, false), Some(1));
        // Unicode 引号容错
        assert_eq!(seek_sequence(&file, &to_vec(&["    println!(“hi”);"]), 0, false), Some(2));
    }
}
