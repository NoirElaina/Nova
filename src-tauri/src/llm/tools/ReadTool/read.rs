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

- Reads text files with optional line range via `offset` and `limit`.
- Reads image files (PNG, JPG, JPEG) — returns base64-encoded content.
- Reads PDF files via the `pages` parameter (e.g. "1-5", max 20 pages/request).
- `file_path` must be an absolute path.
- `offset` is 0-based (line 0 is the first line). When specified, `limit` is required by Claude Code; when both are omitted, the entire file is returned.
- Reading a directory, a missing file, or an empty file returns an error."#
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
                },
                "pages": {
                    "type": "string",
                    "description": "Page range for PDF files (e.g., \"1-5\", \"3\", \"10-20\"). Only applicable to PDF files. Maximum 20 pages per request."
                }
            },
            "required": ["file_path"]
        }),
    }
}

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];
const PDF_EXTENSION: &str = "pdf";
const MAX_TEXT_SIZE: u64 = 2 * 1024 * 1024;

fn ext_lower(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_image_ext(path: &std::path::Path) -> bool {
    IMAGE_EXTENSIONS.contains(&ext_lower(path).as_str())
}

fn is_pdf(path: &std::path::Path) -> bool {
    ext_lower(path) == PDF_EXTENSION
}

fn read_image(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Error reading image {}: {}", path.display(), e))?;
    let mime = match ext_lower(path).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        _ => "image/png",
    };
    let encoded = base64(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn read_text(
    path: &std::path::Path,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Error accessing file: {}", e))?;
    if !metadata.is_file() {
        return Err(format!("Not a regular file: {}", path.display()));
    }
    if metadata.len() > MAX_TEXT_SIZE {
        return Err(format!(
            "File is too large ({} bytes, max {} bytes). Use offset/limit to read in chunks.",
            metadata.len(),
            MAX_TEXT_SIZE
        ));
    }

    let content = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            format!(
                "File does not appear to be valid UTF-8: {}. Try reading it as an image or PDF instead.",
                path.display()
            )
        } else {
            format!("Error reading {}: {}", path.display(), e)
        }
    })?;

    if content.is_empty() {
        return Err(format!("File is empty: {}", path.display()));
    }

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

    let mut output = String::new();
    for (i, line) in shown.iter().enumerate() {
        let line_num = start + i;
        output.push_str(&format!("{:>6}\t{}\n", line_num + 1, line));
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

fn parse_page_range(pages: &str) -> Result<(u32, u32), String> {
    let trimmed = pages.trim();
    if trimmed.is_empty() {
        return Err("pages must not be empty".to_string());
    }
    if let Some((start, end)) = trimmed.split_once('-') {
        let start: u32 = start
            .trim()
            .parse()
            .map_err(|_| format!("Invalid page number: {}", start))?;
        let end: u32 = end
            .trim()
            .parse()
            .map_err(|_| format!("Invalid page number: {}", end))?;
        if start < 1 {
            return Err("Page numbers start at 1".to_string());
        }
        if end < start {
            return Err(format!(
                "Invalid page range: end ({}) must be >= start ({})",
                end, start
            ));
        }
        if end - start + 1 > 20 {
            return Err(format!(
                "Maximum 20 pages per request, requested {}",
                end - start + 1
            ));
        }
        Ok((start, end))
    } else {
        let page: u32 = trimmed
            .parse()
            .map_err(|_| format!("Invalid page number: {}", trimmed))?;
        if page < 1 {
            return Err("Page numbers start at 1".to_string());
        }
        Ok((page, page))
    }
}

fn read_pdf(path: &std::path::Path, pages: Option<&str>) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Error reading PDF {}: {}", path.display(), e))?;

    let doc = lopdf::Document::load_mem(&bytes)
        .map_err(|e| format!("Failed to parse PDF: {}", e))?;

    let total_pages = doc.max_id as u32;

    let (start, end) = if let Some(pages_str) = pages {
        parse_page_range(pages_str)?
    } else if total_pages > 10 {
        return Err(format!(
            "PDF has {} pages. Use the `pages` parameter to specify which pages to read (max 20 per request).",
            total_pages
        ));
    } else {
        (1, total_pages.min(20))
    };

    if start > total_pages {
        return Err(format!(
            "Page {} requested but PDF only has {} pages",
            start, total_pages
        ));
    }
    let end = end.min(total_pages);

    let mut output = String::new();
    for page_num in start..=end {
        output.push_str(&format!("\n--- Page {} ---\n", page_num));
        match doc.extract_text(&[page_num]) {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    output.push_str("[no extractable text on this page]\n");
                } else {
                    output.push_str(trimmed);
                    output.push('\n');
                }
            }
            Err(_) => {
                output.push_str("[could not extract text from this page]\n");
            }
        }
    }

    if end < total_pages {
        output.push_str(&format!(
            "\n... (pages {}-{} of {}, {} pages remaining)\n",
            start,
            end,
            total_pages,
            total_pages - end
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

    let pages = input
        .get("pages")
        .and_then(Value::as_str);

    let path = resolve_file_path(file_path)
        .map_err(|e| ToolFailure::invalid_input(e))?;

    if path.is_dir() {
        return Err(ToolFailure::new(format!(
            "Path is a directory, not a file: {}",
            file_path
        )));
    }

    let result = if is_pdf(&path) {
        read_pdf(&path, pages)
    } else if is_image_ext(&path) {
        read_image(&path)
    } else {
        read_text(&path, limit, offset)
    };

    match result {
        Ok(content) => Ok(ToolOutcome::text(content)),
        Err(e) => Err(ToolFailure::new(e)),
    }
}

fn execute_with_app_boxed(
    _app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(input).await })
}
