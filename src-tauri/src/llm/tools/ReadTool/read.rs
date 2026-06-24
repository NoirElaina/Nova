use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "Read".into(),
        description: r#"Read a file from the local filesystem. Returns the file content with line numbers (like `cat -n`).

- Supports reading text files with optional line range via `offset` and `limit`.
- Supports reading image files (PNG, JPG, JPEG) — returns base64-encoded content.
- `file_path` must be an absolute path.
- `offset` is 0-based (line 0 is the first line). When specified, `limit` is required; when both are omitted, the entire file is returned."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to read"
                },
                "limit": {
                    "type": "integer",
                    "description": "The number of lines to read. Only provide if the file is too large to read at once."
                },
                "offset": {
                    "type": "integer",
                    "description": "The line number to start reading from (0-based). Only provide if the file is too large to read at once."
                }
            },
            "required": ["file_path"]
        }),
    }
}

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];
const MAX_TEXT_SIZE: u64 = 2 * 1024 * 1024;
const MAX_LINES_WITHOUT_PAGING: usize = 2000;

fn is_image_ext(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn read_image(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Error reading image {}: {}", path.display(), e))?;
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let mime = if ext == "jpg" || ext == "jpeg" {
        "image/jpeg"
    } else {
        "image/png"
    };
    let encoded = base64(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn read_text(path: &std::path::Path, limit: Option<usize>, offset: Option<usize>) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Error accessing file: {}", e))?;
    if metadata.len() > MAX_TEXT_SIZE {
        return Err(format!(
            "File is too large ({} bytes, max {} bytes). Use offset/limit to read in chunks.",
            metadata.len(),
            MAX_TEXT_SIZE
        ));
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Error reading {}: {}", path.display(), e))?;

    let all_lines: Vec<&str> = content.lines().collect();
    let total_lines = all_lines.len();

    let start = offset.unwrap_or(0);
    let end = match limit {
        Some(n) => (start + n).min(total_lines),
        None => total_lines,
    };

    if start >= total_lines {
        return Err(format!(
            "offset {} is beyond file end ({} lines)",
            start, total_lines
        ));
    }

    let shown = &all_lines[start..end];
    let show_line_numbers = limit.is_some() || total_lines > MAX_LINES_WITHOUT_PAGING;

    let mut output = String::new();
    for (i, line) in shown.iter().enumerate() {
        let line_num = start + i;
        if show_line_numbers {
            output.push_str(&format!("{:>6}\t{}\n", line_num + 1, line));
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    if limit.is_some() && end < total_lines {
        output.push_str(&format!(
            "\n... (lines {}-{} of {}, {} lines remaining)\n",
            start + 1,
            end,
            total_lines,
            total_lines - end
        ));
    }

    Ok(output)
}

fn resolve_file_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("file_path is required".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!(
            "file_path must be an absolute path: {}",
            trimmed
        ));
    }
    if !path.exists() {
        return Err(format!("File not found: {}", trimmed));
    }
    Ok(path)
}

async fn execute_async(input: Value) -> Result<ToolOutcome, ToolFailure> {
    let file_path = input
        .get("file_path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: file_path"))?;

    let offset = input
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let path = resolve_file_path(file_path)
        .map_err(|e| ToolFailure::invalid_input(e))?;

    if is_image_ext(&path) {
        let result = read_image(&path)
            .map_err(|e| ToolFailure::new(e))?;
        Ok(ToolOutcome::text(result))
    } else {
        let result = read_text(&path, limit, offset)
            .map_err(|e| ToolFailure::new(e))?;
        Ok(ToolOutcome::text(result))
    }
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(input).await })
}
