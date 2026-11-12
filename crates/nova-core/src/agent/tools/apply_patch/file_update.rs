//! 差异计算、文件原子更新与 Unified Diff 生成。

use similar::TextDiff;
use thiserror::Error;

use super::parser::UpdateFileChunk;
use super::seek_sequence;
use super::text_file::{Replacement, SourceFile};

#[derive(Debug, Error)]
pub enum ApplyPatchError {
    #[error("Failed to compute replacements: {0}")]
    ComputeReplacements(String),
    #[error("I/O error during patch application: {0}")]
    IoError(String),
}

pub struct AppliedFileDiff {
    pub original_content: String,
    pub new_content: String,
    pub unified_diff: String,
}

/// 计算从 `original_lines` 转换到新内容所需要的区间替换序列。
/// 每一条替换记录为 `(start_index, old_len, new_lines)`。
fn compute_replacements(
    original_lines: &[String],
    path_display: &str,
    chunks: &[UpdateFileChunk],
) -> Result<Vec<Replacement>, ApplyPatchError> {
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut line_index: usize = 0;

    for chunk in chunks {
        // 若指定了 change_context，使用 seek_sequence 定位上下文锚点行
        if let Some(ctx_line) = &chunk.change_context {
            let found = seek_sequence::seek_sequence(
                original_lines,
                std::slice::from_ref(ctx_line),
                line_index,
                false,
            )
            .or_else(|| {
                // 宽容容错：若未完全整行匹配，检查是否有行包含该上下文标记（如 'fn main()' 匹配 'fn main() {'）
                let trimmed_ctx = ctx_line.trim();
                if !trimmed_ctx.is_empty() {
                    for (i, line) in original_lines.iter().enumerate().skip(line_index) {
                        if line.contains(trimmed_ctx) {
                            return Some(i);
                        }
                    }
                }
                None
            });

            if let Some(idx) = found {
                line_index = idx + 1;
            } else {
                return Err(ApplyPatchError::ComputeReplacements(format!(
                    "Failed to find context '{ctx_line}' in {path_display}"
                )));
            }
        }

        if chunk.old_lines.is_empty() {
            replacements.push((original_lines.len(), 0, chunk.new_lines.clone()));
            continue;
        }

        // 尝试定位 old_lines 序列
        let mut pattern: &[String] = &chunk.old_lines;
        let mut found =
            seek_sequence::seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        let mut new_slice: &[String] = &chunk.new_lines;

        // 若直接匹配失败，且末尾为空行（代表换行哨兵），去除末尾空行后重试
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence::seek_sequence(
                original_lines,
                pattern,
                line_index,
                chunk.is_end_of_file,
            );
        }

        if let Some(start_idx) = found {
            // 保留两端相同的上下文行不被替换，只替换真正变化的行
            let mut old_start = 0;
            let mut new_start = 0;
            for &(old_context, new_context) in &chunk.context_line_indices {
                if old_context >= pattern.len() || new_context >= new_slice.len() {
                    break;
                }
                if old_start != old_context || new_start != new_context {
                    replacements.push((
                        start_idx + old_start,
                        old_context - old_start,
                        new_slice[new_start..new_context].to_vec(),
                    ));
                }
                old_start = old_context + 1;
                new_start = new_context + 1;
            }
            if old_start != pattern.len() || new_start != new_slice.len() {
                replacements.push((
                    start_idx + old_start,
                    pattern.len() - old_start,
                    new_slice[new_start..].to_vec(),
                ));
            }
            line_index = start_idx + pattern.len();
        } else {
            return Err(ApplyPatchError::ComputeReplacements(format!(
                "Failed to find expected lines in {}:\n{}",
                path_display,
                chunk.old_lines.join("\n"),
            )));
        }
    }

    replacements.sort_by_key(|(index, _, _)| *index);
    Ok(replacements)
}

/// 对文本应用 chunks，返回更新后的完整文本与 unified diff。
pub fn derive_new_contents_from_chunks(
    original_contents: &str,
    path_display: &str,
    chunks: &[UpdateFileChunk],
) -> Result<AppliedFileDiff, ApplyPatchError> {
    let mut source_file = SourceFile::parse(original_contents);
    let original_lines = source_file.line_texts();
    let replacements = compute_replacements(&original_lines, path_display, chunks)?;
    source_file.apply_replacements(&replacements);
    let new_content = source_file.into_contents();

    let text_diff = TextDiff::from_lines(original_contents, &new_content);
    let unified_diff = text_diff.unified_diff().context_radius(2).to_string();

    Ok(AppliedFileDiff {
        original_content: original_contents.to_string(),
        new_content,
        unified_diff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_update_content() {
        let original = "fn main() {\n    println!(\"old\");\n}\n";
        let chunk = UpdateFileChunk {
            change_context: Some("fn main()".to_string()),
            old_lines: vec!["    println!(\"old\");".to_string()],
            new_lines: vec!["    println!(\"new\");".to_string()],
            context_line_indices: vec![],
            is_end_of_file: false,
        };

        let res = derive_new_contents_from_chunks(original, "test.rs", &[chunk]).unwrap();
        assert_eq!(res.new_content, "fn main() {\n    println!(\"new\");\n}\n");
        assert!(res.unified_diff.contains("-    println!(\"old\");"));
        assert!(res.unified_diff.contains("+    println!(\"new\");"));
    }
}
