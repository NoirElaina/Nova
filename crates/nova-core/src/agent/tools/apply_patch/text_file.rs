//! 具备行尾保真特性的源码文件抽象。
//!
//! 在编辑时保留未改动行的原始换行符（CRLF/LF），新插入行使用文件首选风格，
//! 杜绝在 Windows 上破坏换行格式。

pub(super) type Replacement = (usize, usize, Vec<String>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

pub struct SourceLine {
    pub text: String,
    pub ending: Option<LineEnding>,
}

pub struct SourceFile {
    pub lines: Vec<SourceLine>,
    pub preferred_ending: LineEnding,
}

impl SourceFile {
    /// 将文本拆解为逻辑行，并记录每行的原始换行符号。
    /// 第一行存在的换行符作为插入行的首选风格；若没有换行符则默认 LF。
    pub fn parse(contents: &str) -> Self {
        let mut lines = Vec::new();
        let mut preferred_ending = None;
        let mut line_start = 0;
        let mut cursor = 0;

        while cursor < contents.len() {
            let (ending, ending_len) = match contents.as_bytes()[cursor] {
                b'\r' if contents.as_bytes().get(cursor + 1) == Some(&b'\n') => {
                    (LineEnding::CrLf, 2)
                }
                b'\r' => (LineEnding::Cr, 1),
                b'\n' => (LineEnding::Lf, 1),
                _ => {
                    cursor += 1;
                    continue;
                }
            };
            preferred_ending.get_or_insert(ending);
            lines.push(SourceLine {
                text: contents[line_start..cursor].to_string(),
                ending: Some(ending),
            });
            cursor += ending_len;
            line_start = cursor;
        }

        if line_start < contents.len() {
            lines.push(SourceLine {
                text: contents[line_start..].to_string(),
                ending: None,
            });
        }

        Self {
            lines,
            preferred_ending: preferred_ending.unwrap_or(LineEnding::Lf),
        }
    }

    pub fn line_texts(&self) -> Vec<String> {
        self.lines.iter().map(|line| line.text.clone()).collect()
    }

    /// 根据按源文件行序排列的无重叠替换操作重新组装文件。
    pub fn apply_replacements(&mut self, replacements: &[Replacement]) {
        let mut source_lines = std::mem::take(&mut self.lines).into_iter();
        let mut new_lines = Vec::new();
        let mut source_index = 0;

        for (start_idx, old_len, new_segment) in replacements {
            debug_assert!(*start_idx >= source_index);
            for line in source_lines.by_ref().take(*start_idx - source_index) {
                new_lines.push(line);
            }
            for _ in source_lines.by_ref().take(*old_len) {}
            new_lines.extend(new_segment.iter().map(|text| SourceLine {
                text: text.clone(),
                ending: Some(self.preferred_ending),
            }));
            source_index = start_idx + old_len;
        }

        new_lines.extend(source_lines);
        self.lines = new_lines;

        for line in &mut self.lines {
            line.ending.get_or_insert(self.preferred_ending);
        }
    }

    pub fn into_contents(self) -> String {
        let mut contents = String::new();
        for line in self.lines {
            contents.push_str(&line.text);
            if let Some(ending) = line.ending {
                contents.push_str(ending.as_str());
            }
        }
        contents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crlf_preservation() {
        let source = "line1\r\nline2\r\nline3\r\n";
        let mut file = SourceFile::parse(source);
        assert_eq!(file.preferred_ending, LineEnding::CrLf);
        file.apply_replacements(&[(1, 1, vec!["modified_line2".to_string()])]);
        let result = file.into_contents();
        assert_eq!(result, "line1\r\nmodified_line2\r\nline3\r\n");
    }
}
