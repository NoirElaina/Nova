//! 词法与语法解析器，支持流式解析与严格/宽容边界检查。

use std::path::{Path, PathBuf};
use thiserror::Error;

pub const BEGIN_PATCH_MARKER: &str = "*** Begin Patch";
pub const END_PATCH_MARKER: &str = "*** End Patch";
pub const ADD_FILE_MARKER: &str = "*** Add File: ";
pub const DELETE_FILE_MARKER: &str = "*** Delete File: ";
pub const UPDATE_FILE_MARKER: &str = "*** Update File: ";
pub const MOVE_TO_MARKER: &str = "*** Move to: ";
pub const EOF_MARKER: &str = "*** End of File";
pub const CHANGE_CONTEXT_MARKER: &str = "@@ ";
pub const EMPTY_CHANGE_CONTEXT_MARKER: &str = "@@";

#[derive(Debug, PartialEq, Error, Clone)]
pub enum ParseError {
    #[error("Invalid patch: {0}")]
    InvalidPatchError(String),
    #[error("Invalid hunk at line {line_number}: {message}")]
    InvalidHunkError { message: String, line_number: usize },
}

#[derive(Debug, PartialEq, Clone)]
pub enum Hunk {
    AddFile {
        path: PathBuf,
        contents: String,
    },
    DeleteFile {
        path: PathBuf,
    },
    UpdateFile {
        path: PathBuf,
        move_path: Option<PathBuf>,
        chunks: Vec<UpdateFileChunk>,
    },
}

impl Hunk {
    pub fn path(&self) -> &Path {
        match self {
            Hunk::AddFile { path, .. } => path,
            Hunk::DeleteFile { path } => path,
            Hunk::UpdateFile { path, .. } => path,
        }
    }

    pub fn paths(&self) -> Vec<&Path> {
        match self {
            Hunk::AddFile { path, .. } | Hunk::DeleteFile { path } => vec![path],
            Hunk::UpdateFile {
                path,
                move_path: Some(dest),
                ..
            } => vec![path, dest],
            Hunk::UpdateFile { path, move_path: None, .. } => vec![path],
        }
    }
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct UpdateFileChunk {
    pub change_context: Option<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub context_line_indices: Vec<(usize, usize)>,
    pub is_end_of_file: bool,
}

impl UpdateFileChunk {
    pub fn push_context_line(&mut self, line: String) {
        self.context_line_indices
            .push((self.old_lines.len(), self.new_lines.len()));
        self.old_lines.push(line.clone());
        self.new_lines.push(line);
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct ApplyPatchArgs {
    pub patch: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ParserMode {
    #[default]
    NotStarted,
    StartedPatch,
    AddFile,
    DeleteFile,
    UpdateFile {
        hunk_line_number: usize,
    },
    EndedPatch,
}

#[derive(Debug, Default, Clone)]
pub struct StreamingPatchParser {
    line_buffer: String,
    mode: ParserMode,
    hunks: Vec<Hunk>,
    line_number: usize,
}

impl StreamingPatchParser {
    fn ensure_update_hunk_is_not_empty(&self, line: &str) -> Result<(), ParseError> {
        if let Some(Hunk::UpdateFile { path, chunks, .. }) = self.hunks.last() {
            if chunks.is_empty() {
                if let ParserMode::UpdateFile { hunk_line_number } = self.mode {
                    return Err(ParseError::InvalidHunkError {
                        message: format!("Update file hunk for path '{}' is empty", path.display()),
                        line_number: hunk_line_number,
                    });
                }
            }
            if chunks
                .last()
                .is_some_and(|chunk| chunk.old_lines.is_empty() && chunk.new_lines.is_empty())
            {
                if line == END_PATCH_MARKER {
                    return Err(ParseError::InvalidHunkError {
                        message: "Update hunk does not contain any lines".to_string(),
                        line_number: self.line_number,
                    });
                }
                return Err(ParseError::InvalidHunkError {
                    message: format!(
                        "Unexpected line found in update hunk: '{line}'. Every line should start with ' ' (context line), '+' (added line), or '-' (removed line)"
                    ),
                    line_number: self.line_number,
                });
            }
        }
        Ok(())
    }

    fn handle_hunk_headers_and_end_patch(&mut self, trimmed: &str) -> Result<bool, ParseError> {
        if trimmed == END_PATCH_MARKER {
            self.ensure_update_hunk_is_not_empty(trimmed)?;
            self.mode = ParserMode::EndedPatch;
            return Ok(true);
        }
        if let Some(path) = trimmed.strip_prefix(ADD_FILE_MARKER) {
            self.ensure_update_hunk_is_not_empty(trimmed)?;
            self.hunks.push(Hunk::AddFile {
                path: PathBuf::from(path.trim()),
                contents: String::new(),
            });
            self.mode = ParserMode::AddFile;
            return Ok(true);
        }
        if let Some(path) = trimmed.strip_prefix(DELETE_FILE_MARKER) {
            self.ensure_update_hunk_is_not_empty(trimmed)?;
            self.hunks.push(Hunk::DeleteFile {
                path: PathBuf::from(path.trim()),
            });
            self.mode = ParserMode::DeleteFile;
            return Ok(true);
        }
        if let Some(path) = trimmed.strip_prefix(UPDATE_FILE_MARKER) {
            self.ensure_update_hunk_is_not_empty(trimmed)?;
            self.hunks.push(Hunk::UpdateFile {
                path: PathBuf::from(path.trim()),
                move_path: None,
                chunks: Vec::new(),
            });
            self.mode = ParserMode::UpdateFile {
                hunk_line_number: self.line_number,
            };
            return Ok(true);
        }
        Ok(false)
    }

    pub fn push_delta(&mut self, delta: &str) -> Result<Vec<Hunk>, ParseError> {
        for ch in delta.chars() {
            if ch == '\n' {
                let mut line = std::mem::take(&mut self.line_buffer);
                line.truncate(line.strip_suffix('\r').map_or(line.len(), str::len));
                self.line_number += 1;
                self.process_line(&line)?;
            } else {
                self.line_buffer.push(ch);
            }
        }
        Ok(self.hunks.clone())
    }

    pub fn finish(&mut self) -> Result<Vec<Hunk>, ParseError> {
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            self.line_number += 1;
            if line.trim() == END_PATCH_MARKER {
                self.ensure_update_hunk_is_not_empty(line.trim())?;
                self.mode = ParserMode::EndedPatch;
            } else {
                self.process_line(&line)?;
            }
        }

        if !matches!(self.mode, ParserMode::EndedPatch) {
            return Err(ParseError::InvalidPatchError(
                "The last line of the patch must be '*** End Patch'".to_string(),
            ));
        }

        Ok(self.hunks.clone())
    }

    fn process_line(&mut self, line: &str) -> Result<(), ParseError> {
        let trimmed = line.trim();
        match self.mode {
            ParserMode::NotStarted => {
                if trimmed == BEGIN_PATCH_MARKER {
                    self.mode = ParserMode::StartedPatch;
                    return Ok(());
                }
                Err(ParseError::InvalidPatchError(
                    "The first line of the patch must be '*** Begin Patch'".to_string(),
                ))
            }
            ParserMode::StartedPatch => {
                if self.handle_hunk_headers_and_end_patch(trimmed)? {
                    return Ok(());
                }
                Err(ParseError::InvalidHunkError {
                    message: format!(
                        "'{trimmed}' is not a valid hunk header. Valid headers: '*** Add File: {{path}}', '*** Delete File: {{path}}', '*** Update File: {{path}}'"
                    ),
                    line_number: self.line_number,
                })
            }
            ParserMode::AddFile => {
                if self.handle_hunk_headers_and_end_patch(trimmed)? {
                    return Ok(());
                }
                if let Some(line_to_add) = line.strip_prefix('+') {
                    if let Some(Hunk::AddFile { contents, .. }) = self.hunks.last_mut() {
                        contents.push_str(line_to_add);
                        contents.push('\n');
                        return Ok(());
                    }
                }
                Err(ParseError::InvalidHunkError {
                    message: format!("Unexpected line in AddFile: '{line}'"),
                    line_number: self.line_number,
                })
            }
            ParserMode::DeleteFile => {
                if self.handle_hunk_headers_and_end_patch(trimmed)? {
                    return Ok(());
                }
                Err(ParseError::InvalidHunkError {
                    message: format!("Unexpected line in DeleteFile: '{line}'"),
                    line_number: self.line_number,
                })
            }
            ParserMode::UpdateFile { hunk_line_number } => {
                let update_line = line.trim_end();
                if self.handle_hunk_headers_and_end_patch(update_line)? {
                    return Ok(());
                }

                if let Some(Hunk::UpdateFile {
                    move_path, chunks, ..
                }) = self.hunks.last_mut()
                {
                    if chunks.last().is_some_and(|chunk| chunk.is_end_of_file) {
                        if update_line.is_empty() {
                            return Ok(());
                        }
                        if update_line != EMPTY_CHANGE_CONTEXT_MARKER
                            && !update_line.starts_with(CHANGE_CONTEXT_MARKER)
                        {
                            return Err(ParseError::InvalidHunkError {
                                message: format!(
                                    "Expected update hunk to start with a @@ context marker, got: '{line}'"
                                ),
                                line_number: self.line_number,
                            });
                        }
                    }

                    if chunks.is_empty() && move_path.is_none() {
                        if let Some(move_to_path) = update_line.strip_prefix(MOVE_TO_MARKER) {
                            *move_path = Some(PathBuf::from(move_to_path.trim()));
                            self.mode = ParserMode::UpdateFile { hunk_line_number };
                            return Ok(());
                        }
                    }

                    if (update_line == EMPTY_CHANGE_CONTEXT_MARKER
                        || update_line.starts_with(CHANGE_CONTEXT_MARKER))
                        && chunks.last().is_some_and(|chunk| {
                            chunk.old_lines.is_empty() && chunk.new_lines.is_empty()
                        })
                    {
                        return Err(ParseError::InvalidHunkError {
                            message: format!(
                                "Unexpected line found in update hunk: '{line}'. Every line should start with ' ' (context line), '+' (added line), or '-' (removed line)"
                            ),
                            line_number: self.line_number,
                        });
                    }

                    if update_line == EMPTY_CHANGE_CONTEXT_MARKER {
                        chunks.push(UpdateFileChunk::default());
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    if let Some(change_context) = update_line.strip_prefix(CHANGE_CONTEXT_MARKER) {
                        let cleaned = change_context.trim_end_matches("@@").trim();
                        chunks.push(UpdateFileChunk {
                            change_context: if cleaned.is_empty() {
                                None
                            } else {
                                Some(cleaned.to_string())
                            },
                            ..UpdateFileChunk::default()
                        });
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    if update_line == EOF_MARKER {
                        if chunks.last().is_some_and(|chunk| {
                            chunk.old_lines.is_empty() && chunk.new_lines.is_empty()
                        }) {
                            return Err(ParseError::InvalidHunkError {
                                message: "Update hunk does not contain any lines".to_string(),
                                line_number: self.line_number,
                            });
                        }
                        if let Some(chunk) = chunks.last_mut() {
                            chunk.is_end_of_file = true;
                        }
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    if line.is_empty() {
                        if chunks.is_empty() {
                            chunks.push(UpdateFileChunk::default());
                        }
                        if let Some(chunk) = chunks.last_mut() {
                            chunk.push_context_line(String::new());
                        }
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    if let Some(line_to_add) = line.strip_prefix(' ') {
                        if chunks.is_empty() {
                            chunks.push(UpdateFileChunk::default());
                        }
                        if let Some(chunk) = chunks.last_mut() {
                            chunk.push_context_line(line_to_add.to_string());
                        }
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    if let Some(line_to_add) = line.strip_prefix('+') {
                        if chunks.is_empty() {
                            chunks.push(UpdateFileChunk::default());
                        }
                        if let Some(chunk) = chunks.last_mut() {
                            chunk.new_lines.push(line_to_add.to_string());
                        }
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    if let Some(line_to_remove) = line.strip_prefix('-') {
                        if chunks.is_empty() {
                            chunks.push(UpdateFileChunk::default());
                        }
                        if let Some(chunk) = chunks.last_mut() {
                            chunk.old_lines.push(line_to_remove.to_string());
                        }
                        self.mode = ParserMode::UpdateFile { hunk_line_number };
                        return Ok(());
                    }

                    Err(ParseError::InvalidHunkError {
                        message: format!(
                            "Unexpected line found in update hunk: '{line}'. Lines should start with ' ' (context), '+' (add), or '-' (remove)"
                        ),
                        line_number: self.line_number,
                    })
                } else {
                    Err(ParseError::InvalidHunkError {
                        message: "UpdateFile state without active hunk".to_string(),
                        line_number: self.line_number,
                    })
                }
            }
            ParserMode::EndedPatch => {
                if trimmed.is_empty() {
                    Ok(())
                } else {
                    Err(ParseError::InvalidPatchError(format!(
                        "Unexpected content after '*** End Patch': '{line}'"
                    )))
                }
            }
        }
    }
}

/// 解析补丁文本（包含宽容模式，自动剥除模型包裹的 <<'EOF' ... EOF 标记）。
pub fn parse_patch(patch: &str) -> Result<ApplyPatchArgs, ParseError> {
    let raw = patch.trim();
    let lines: Vec<&str> = raw.lines().map(|l| l.trim_end_matches('\r')).collect();

    // 宽容模式：剥离模型偶尔输出的外层 shell heredoc
    let patch_lines = match lines.as_slice() {
        [first, .., last]
            if (first.starts_with("<<EOF") || first.starts_with("<<'EOF'") || first.starts_with("<<\"EOF\""))
                && last.trim().ends_with("EOF")
                && lines.len() >= 4 =>
        {
            &lines[1..lines.len() - 1]
        }
        _ => lines.as_slice(),
    };

    let normalized = patch_lines.join("\n");
    let mut parser = StreamingPatchParser::default();
    parser.push_delta(&normalized)?;
    let hunks = parser.finish()?;

    Ok(ApplyPatchArgs {
        patch: normalized,
        hunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_update_patch() {
        let patch = r#"*** Begin Patch
*** Update File: src/lib.rs
@@ fn test() @@
-    let a = 1;
+    let a = 2;
*** End Patch"#;

        let res = parse_patch(patch).expect("valid patch should parse");
        assert_eq!(res.hunks.len(), 1);
        if let Hunk::UpdateFile { path, chunks, .. } = &res.hunks[0] {
            assert_eq!(path, Path::new("src/lib.rs"));
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].change_context.as_deref(), Some("fn test()"));
            assert_eq!(chunks[0].old_lines, vec!["    let a = 1;"]);
            assert_eq!(chunks[0].new_lines, vec!["    let a = 2;"]);
        } else {
            panic!("expected UpdateFile");
        }
    }

    #[test]
    fn test_parse_multi_file_patch() {
        let patch = r#"*** Begin Patch
*** Add File: new.txt
+hello world
*** Delete File: old.txt
*** Update File: main.rs
@@ @@
-foo
+bar
*** End Patch"#;

        let res = parse_patch(patch).expect("valid multi-file patch");
        assert_eq!(res.hunks.len(), 3);
        assert!(matches!(res.hunks[0], Hunk::AddFile { .. }));
        assert!(matches!(res.hunks[1], Hunk::DeleteFile { .. }));
        assert!(matches!(res.hunks[2], Hunk::UpdateFile { .. }));
    }
}
