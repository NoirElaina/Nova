use serde_json::{json, Value};
use crate::agent::utils::file_io::{read_file_meta, resolve_tool_path, write_file_with_meta};

#[derive(Debug)]
pub struct EditResult {
    pub file_path: String,
    pub occurrences_replaced: usize,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct EditOperation {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

/// 尝试在 original 中寻找与 old_string 在忽略空白差异后唯一匹配的行区间
/// 返回匹配的原始切片 `(start_byte, end_byte, line_start_1_indexed)`
fn find_trimmed_match_range(original: &str, old_string: &str) -> Option<(usize, usize, usize)> {
    let target_ends_with_newline = old_string.ends_with('\n');
    let target_body = old_string.strip_suffix('\n').unwrap_or(old_string);
    let target_lines: Vec<&str> = target_body.split('\n').collect();
    if target_lines.is_empty() {
        return None;
    }

    // 构建 original 的行信息：(行文本, start_byte, end_byte)
    let mut orig_lines = Vec::new();
    let mut offset = 0;
    for line in original.split('\n') {
        let start = offset;
        let end = offset + line.len();
        orig_lines.push((line, start, end));
        offset = end + 1; // +1 for '\n'
    }

    // 如果 original 末尾有 '\n'，split('\n') 最后一个元素是 "" 且代表 EOF，不作为实际代码行
    if original.ends_with('\n') && orig_lines.last().map(|(l, _, _)| *l == "").unwrap_or(false) {
        orig_lines.pop();
    }

    if orig_lines.len() < target_lines.len() {
        return None;
    }

    // 优先：仅忽略行尾空白（trim_end），保留原行缩进
    let mut matches = Vec::new();
    for i in 0..=(orig_lines.len() - target_lines.len()) {
        let mut matched = true;
        for j in 0..target_lines.len() {
            if orig_lines[i + j].0.trim_end() != target_lines[j].trim_end() {
                matched = false;
                break;
            }
        }
        if matched {
            let start_byte = orig_lines[i].1;
            let last_idx = i + target_lines.len() - 1;
            let end_byte = if target_ends_with_newline {
                (orig_lines[last_idx].2 + 1).min(original.len())
            } else {
                orig_lines[last_idx].2
            };
            matches.push((start_byte, end_byte, i + 1));
        }
    }

    if matches.len() == 1 {
        return Some(matches[0]);
    }

    // 次选：两端均 trim（容忍缩进差异）
    if matches.is_empty() {
        let mut trim_both_matches = Vec::new();
        for i in 0..=(orig_lines.len() - target_lines.len()) {
            let mut matched = true;
            for j in 0..target_lines.len() {
                if orig_lines[i + j].0.trim() != target_lines[j].trim() {
                    matched = false;
                    break;
                }
            }
            if matched {
                let start_byte = orig_lines[i].1;
                let last_idx = i + target_lines.len() - 1;
                let end_byte = if target_ends_with_newline {
                    (orig_lines[last_idx].2 + 1).min(original.len())
                } else {
                    orig_lines[last_idx].2
                };
                trim_both_matches.push((start_byte, end_byte, i + 1));
            }
        }
        if trim_both_matches.len() == 1 {
            return Some(trim_both_matches[0]);
        }
    }

    None
}

/// 在原始文件中寻找与 target 相似度最高的区域，格式化为带行号的诊断代码块
fn format_closest_match_diagnostic(original: &str, target: &str, file_path: &str) -> String {
    let original_lines: Vec<&str> = original.lines().collect();
    if original_lines.is_empty() {
        return format!("Could not find exact match for old_string in {} (file is empty).", file_path);
    }

    let target_lines: Vec<&str> = target.lines().collect();
    let first_target_trimmed = target_lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim());

    let mut best_line = None;

    if let Some(first_target) = first_target_trimmed {
        // 1. 寻找包含首行子串的行
        for (idx, line) in original_lines.iter().enumerate() {
            let trimmed_line = line.trim();
            if trimmed_line == first_target
                || trimmed_line.contains(first_target)
                || first_target.contains(trimmed_line)
            {
                best_line = Some(idx);
                break;
            }
        }

        // 2. 统计单词词频重合度最高的行
        if best_line.is_none() {
            let target_words: Vec<&str> = first_target.split_whitespace().collect();
            let mut max_overlap = 0;
            let mut best_idx = 0;
            for (idx, line) in original_lines.iter().enumerate() {
                let count = target_words.iter().filter(|w| line.contains(**w)).count();
                if count > max_overlap {
                    max_overlap = count;
                    best_idx = idx;
                }
            }
            if max_overlap > 0 {
                best_line = Some(best_idx);
            }
        }
    }

    let center_idx = best_line.unwrap_or(0);
    let start_idx = center_idx.saturating_sub(2);
    let end_idx = (center_idx + target_lines.len().max(1) + 2).min(original_lines.len());

    let mut snippet = String::new();
    for i in start_idx..end_idx {
        use std::fmt::Write;
        let _ = writeln!(snippet, "{:>5} | {}", i + 1, original_lines[i]);
    }

    format!(
        "Could not find exact match for old_string in {}.\n\
         Nearest similar content found in file (around line {}):\n\
         --------------------------------------------------\n\
         {}\
         --------------------------------------------------\n\
         Please check indentation, whitespace, or copy the exact lines above to retry.",
        file_path,
        center_idx + 1,
        snippet
    )
}

/// 工业级精确块匹配文件修改引擎（Exact Chunk Replacer with Line-Trimmed Fallback & Nearest-Block Diagnostics）
pub async fn execute_exact_edit(
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<Value, String> {
    if old_string == new_string {
        return Err("old_string and new_string must be different".into());
    }
    if old_string.is_empty() {
        return Err("old_string must not be empty".into());
    }

    let target = resolve_tool_path(file_path).map_err(|e| e.to_string())?;
    if !target.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }
    if target.is_dir() {
        return Err(format!("Path is a directory, not a file: {}", file_path));
    }

    // 读入原始内容并归一化为 LF
    let (original, meta) = read_file_meta(&target)
        .map_err(|e| format!("Error reading {}: {}", file_path, e))?;

    let old_normalized = old_string.replace("\r\n", "\n");
    let new_normalized = new_string.replace("\r\n", "\n");

    let count = original.matches(&old_normalized).count();

    let (modified, replaced_count, start_line, end_line) = if count > 0 {
        if count > 1 && !replace_all {
            return Err(format!(
                "Found {} occurrences of old_string in {}. Please include more surrounding context lines to make the match unique, or specify replace_all=true.",
                count, file_path
            ));
        }

        let first_byte_pos = original.find(&old_normalized).unwrap_or(0);
        let start_line = original[..first_byte_pos].chars().filter(|&c| c == '\n').count() + 1;
        let new_lines_count = new_normalized.chars().filter(|&c| c == '\n').count();
        let end_line = start_line + new_lines_count;

        let modified = if replace_all {
            original.replace(&old_normalized, &new_normalized)
        } else {
            original.replacen(&old_normalized, &new_normalized, 1)
        };

        (modified, if replace_all { count } else { 1 }, start_line, end_line)
    } else {
        // 精确匹配未命中：尝试行尾空白容错自愈
        if let Some((start_byte, end_byte, start_line)) = find_trimmed_match_range(&original, &old_normalized) {
            let mut mod_str = String::with_capacity(original.len() + new_normalized.len());
            mod_str.push_str(&original[..start_byte]);
            mod_str.push_str(&new_normalized);
            mod_str.push_str(&original[end_byte..]);

            let new_lines_count = new_normalized.chars().filter(|&c| c == '\n').count();
            let end_line = start_line + new_lines_count;

            (mod_str, 1, start_line, end_line)
        } else {
            // 自愈依然无法匹配：生成诊断信息返回最近代码块
            return Err(format_closest_match_diagnostic(&original, &old_normalized, file_path));
        }
    };

    // 原子安全写入磁盘
    write_file_with_meta(&target, &modified, &meta)
        .map_err(|e| format!("Failed to write file {}: {}", file_path, e))?;

    Ok(json!({
        "ok": true,
        "file_path": file_path,
        "occurrences_replaced": replaced_count,
        "start_line": start_line,
        "end_line": end_line
    }))
}

/// 工业级批量精确块匹配文件修改引擎（Atomic Multi-Edit Engine）
pub async fn execute_exact_multi_edit(
    file_path: &str,
    edits: &[EditOperation],
) -> Result<Value, String> {
    if edits.is_empty() {
        return Err("edits must not be empty".into());
    }

    let target = resolve_tool_path(file_path).map_err(|e| e.to_string())?;
    if !target.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }
    if target.is_dir() {
        return Err(format!("Path is a directory, not a file: {}", file_path));
    }

    let (mut content, meta) = read_file_meta(&target)
        .map_err(|e| format!("Error reading {}: {}", file_path, e))?;

    let mut applied_count = 0usize;

    for (idx, edit) in edits.iter().enumerate() {
        if edit.old_string == edit.new_string {
            return Err(format!(
                "edits[{}]: old_string and new_string must be different",
                idx
            ));
        }
        if edit.old_string.is_empty() {
            return Err(format!("edits[{}]: old_string must not be empty", idx));
        }

        let old_normalized = edit.old_string.replace("\r\n", "\n");
        let new_normalized = edit.new_string.replace("\r\n", "\n");

        let count = content.matches(&old_normalized).count();
        if count > 0 {
            if count > 1 && !edit.replace_all {
                return Err(format!(
                    "edits[{}]: Found {} occurrences of old_string in {}. Please include more surrounding context lines or set replace_all=true.",
                    idx, count, file_path
                ));
            }

            content = if edit.replace_all {
                content.replace(&old_normalized, &new_normalized)
            } else {
                content.replacen(&old_normalized, &new_normalized, 1)
            };

            applied_count += if edit.replace_all { count } else { 1 };
        } else {
            // 尝试行尾空白容错
            if let Some((start_byte, end_byte, _)) = find_trimmed_match_range(&content, &old_normalized) {
                let mut mod_str = String::with_capacity(content.len() + new_normalized.len());
                mod_str.push_str(&content[..start_byte]);
                mod_str.push_str(&new_normalized);
                mod_str.push_str(&content[end_byte..]);
                content = mod_str;
                applied_count += 1;
            } else {
                let diagnostic = format_closest_match_diagnostic(&content, &old_normalized, file_path);
                return Err(format!("edits[{}]: {}", idx, diagnostic));
            }
        }
    }

    // 全部 edits 成功后原子安全写回磁盘
    write_file_with_meta(&target, &content, &meta)
        .map_err(|e| format!("Failed to write file {}: {}", file_path, e))?;

    Ok(json!({
        "ok": true,
        "file_path": file_path,
        "edits_applied": edits.len(),
        "occurrences_replaced": applied_count
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_trimmed_match_range() {
        let original = "line1\nline2   \nline3\n";
        // Target has no trailing spaces on line2
        let target = "line2\n";
        let res = find_trimmed_match_range(original, target);
        assert!(res.is_some());
        let (start, end, line) = res.unwrap();
        assert_eq!(line, 2);
        assert_eq!(&original[start..end], "line2   \n");
    }

    #[test]
    fn test_closest_match_diagnostic() {
        let original = "fn test() {\n    let a = 1;\n    let b = 2;\n}\n";
        let target = "    let a = 3;";
        let diagnostic = format_closest_match_diagnostic(original, target, "test.rs");
        assert!(diagnostic.contains("Nearest similar content found"));
        assert!(diagnostic.contains("let a = 1;"));
    }
}
