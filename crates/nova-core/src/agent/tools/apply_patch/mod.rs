//! 统一差异补丁合并系统（包含流式解析、渐进序列匹配与原子更新）。

pub mod apply;
pub mod file_update;
pub mod parser;
pub mod seek_sequence;
pub mod text_file;
pub mod tool;

pub use apply::{execute_patch_hunks, PatchExecutionReport};
pub use file_update::{derive_new_contents_from_chunks, AppliedFileDiff, ApplyPatchError};
pub use parser::{parse_patch, ApplyPatchArgs, Hunk, ParseError, UpdateFileChunk};
pub use tool::tool;
