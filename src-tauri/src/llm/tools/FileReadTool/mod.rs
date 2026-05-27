use base64::Engine;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolPermissionDescriptor,
    ToolRegistration,
};
use crate::llm::types::{Content, ContentBlock, ImageSource, Message, Role, Tool};

const DEFAULT_MAX_LINES: usize = 2_000;
const MAX_LIMIT_LINES: usize = 20_000;
const MAX_TEXT_BYTES: u64 = 256 * 1024;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_OUTPUT_TOKENS: usize = 25_000;
const PDF_MAX_PAGES_PER_READ: usize = 20;

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, Some(permission))
}

pub fn tool() -> Tool {
    Tool {
        name: "read_file".into(),
        description: "Read a file from the local filesystem. Use absolute file_path values. Supports ranged text reads, images, and Jupyter notebooks. PDF files are detected and rejected with explicit guidance until Nova supports provider document blocks.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "1-indexed line number to start reading from. 0 is treated as line 1."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT_LINES,
                    "description": "Number of lines to read. Use this for large files or targeted reads."
                },
                "pages": {
                    "type": "string",
                    "description": format!("Page range for PDF files, such as \"1-5\", \"3\", or \"10-20\". Maximum {} pages per request.", PDF_MAX_PAGES_PER_READ)
                }
            },
            "required": ["file_path"],
            "additionalProperties": false
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_async(&app, conversation_id.as_deref(), input).await })
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let raw_path = input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();

    if raw_path.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: "read_file:<empty>".to_string(),
            preview: "读取文件（read_file）：路径为空".to_string(),
            warning: Some("文件路径为空，无法读取。".to_string()),
            needs_approval: false,
        });
    }

    let normalized = normalize_path_for_permission(raw_path);
    let warning = read_permission_warning(raw_path, &normalized);
    Some(ToolPermissionDescriptor {
        signature: format!("read_file:{}", normalized),
        preview: format!("读取文件（read_file）：{}", truncate_chars(raw_path, 220)),
        warning: warning.clone(),
        needs_approval: warning.is_some(),
    })
}

async fn execute_async(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let args = ReadArgs::parse(&input)?;
    let target = expand_absolute_path(&args.file_path).map_err(ToolFailure::invalid_input)?;

    if let Some(pages) = &args.pages {
        validate_pdf_pages(pages).map_err(ToolFailure::invalid_input)?;
    }

    if is_blocked_device_path(&target) {
        return Err(ToolFailure::permission_denied(format!(
            "Cannot read '{}': this device path would block or produce infinite output.",
            target.display()
        )));
    }

    let metadata_path = target.clone();
    let metadata = tokio::task::spawn_blocking(move || std::fs::metadata(&metadata_path))
        .await
        .map_err(|error| ToolFailure::new(format!("read_file metadata worker failed: {}", error)))?
        .map_err(|error| friendly_path_error(&target, error))?;

    if !metadata.is_file() {
        return Err(ToolFailure::new(format!(
            "read_file can only read files, not directories: {}. Use a shell directory listing command to inspect directories.",
            target.display()
        )));
    }

    let ext = file_extension(&target);
    if is_image_extension(&ext) {
        return read_image(target, metadata.len()).await;
    }
    if is_pdf_extension(&ext) {
        return Err(ToolFailure::new(pdf_not_supported_message(
            &target,
            args.pages.as_deref(),
        )));
    }
    if is_binary_extension(&ext) {
        return Err(ToolFailure::new(format!(
            "This tool cannot read binary files. The file appears to be a binary .{} file: {}",
            ext,
            target.display()
        )));
    }
    if ext == "ipynb" {
        return read_notebook(target, metadata.len()).await;
    }

    read_text(target, metadata.len(), args.offset, args.limit).await
}

struct ReadArgs {
    file_path: String,
    offset: usize,
    limit: Option<usize>,
    pages: Option<String>,
}

impl ReadArgs {
    fn parse(input: &Value) -> Result<Self, ToolFailure> {
        let file_path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ToolFailure::invalid_input("Missing 'file_path' argument"))?
            .to_string();

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(1);
        let offset = if offset == 0 { 1 } else { offset };

        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        if matches!(limit, Some(0)) {
            return Err(ToolFailure::invalid_input("'limit' must be greater than 0"));
        }
        if matches!(limit, Some(value) if value > MAX_LIMIT_LINES) {
            return Err(ToolFailure::invalid_input(format!(
                "'limit' cannot exceed {} lines",
                MAX_LIMIT_LINES
            )));
        }

        let pages = input
            .get("pages")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);

        Ok(Self {
            file_path,
            offset,
            limit,
            pages,
        })
    }
}

async fn read_text(
    path: PathBuf,
    file_size: u64,
    offset: usize,
    limit: Option<usize>,
) -> Result<ToolOutcome, ToolFailure> {
    if limit.is_none() && file_size > MAX_TEXT_BYTES {
        return Err(ToolFailure::new(format!(
            "File content ({}) exceeds maximum allowed size ({}). Use offset and limit parameters to read specific portions of the file, or use grep_search to find specific content.",
            format_file_size(file_size),
            format_file_size(MAX_TEXT_BYTES)
        )));
    }

    let path_for_read = path.clone();
    let result =
        tokio::task::spawn_blocking(move || read_text_range(&path_for_read, file_size, offset, limit))
            .await
            .map_err(|error| ToolFailure::new(format!("read_file text worker failed: {}", error)))?
            .map_err(ToolFailure::new)?;

    validate_output_tokens(&result.content)?;
    Ok(ToolOutcome::text(format_text_result(&path, &result)))
}

async fn read_notebook(path: PathBuf, file_size: u64) -> Result<ToolOutcome, ToolFailure> {
    if file_size > MAX_TEXT_BYTES {
        return Err(ToolFailure::new(format!(
            "Notebook content ({}) exceeds maximum allowed size ({}). Use a shell command with jq to inspect targeted cells, for example: jq '.cells[:20]' \"{}\"",
            format_file_size(file_size),
            format_file_size(MAX_TEXT_BYTES),
            path.display()
        )));
    }

    let path_for_read = path.clone();
    let output = tokio::task::spawn_blocking(move || read_notebook_text(&path_for_read))
        .await
        .map_err(|error| ToolFailure::new(format!("read_file notebook worker failed: {}", error)))?
        .map_err(ToolFailure::new)?;

    validate_output_tokens(&output)?;
    Ok(ToolOutcome::text(output))
}

async fn read_image(path: PathBuf, file_size: u64) -> Result<ToolOutcome, ToolFailure> {
    if file_size == 0 {
        return Err(ToolFailure::new(format!(
            "Image file is empty: {}",
            path.display()
        )));
    }
    if file_size > MAX_IMAGE_BYTES {
        return Err(ToolFailure::new(format!(
            "Image file ({}) exceeds maximum supported inline size ({}). Compress or resize it before reading: {}",
            format_file_size(file_size),
            format_file_size(MAX_IMAGE_BYTES),
            path.display()
        )));
    }

    let path_for_read = path.clone();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&path_for_read))
        .await
        .map_err(|error| ToolFailure::new(format!("read_file image worker failed: {}", error)))?
        .map_err(|error| ToolFailure::new(format!("Error reading image file: {}", error)))?;

    let media_type = detect_image_media_type(&path, &bytes).ok_or_else(|| {
        ToolFailure::new(format!(
            "Unsupported or unrecognized image format: {}",
            path.display()
        ))
    })?;

    let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let text = format!(
        "Image file read: {} ({}, {}). The image is attached as additional context.",
        path.display(),
        format_file_size(file_size),
        media_type
    );
    let output = json!({
        "type": "image",
        "file_path": path.display().to_string(),
        "media_type": media_type,
        "original_size": file_size,
        "attached_to_context": true
    })
    .to_string();

    let message = Message {
        role: Role::User,
        content: Content::Blocks(vec![
            ContentBlock::Text { text },
            ContentBlock::Image {
                source: ImageSource {
                    source_type: "base64".to_string(),
                    media_type: media_type.to_string(),
                    data: base64,
                },
            },
        ]),
    };

    Ok(ToolOutcome::text(output).with_additional_messages(vec![message]))
}

struct TextRangeResult {
    content: String,
    selected_lines: Vec<String>,
    start_line: usize,
    num_lines: usize,
    total_lines: Option<usize>,
    has_more: bool,
    limit: usize,
}

fn read_text_range(
    path: &Path,
    file_size: u64,
    offset: usize,
    requested_limit: Option<usize>,
) -> Result<TextRangeResult, String> {
    let limit = requested_limit.unwrap_or(DEFAULT_MAX_LINES);
    let count_all_lines = file_size <= MAX_TEXT_BYTES;
    let start_index = offset.saturating_sub(1);
    let end_index = start_index.saturating_add(limit);
    let file = File::open(path).map_err(|error| format!("Error opening file: {}", error))?;
    let mut reader = BufReader::new(file);
    let mut selected_lines = Vec::new();
    let mut selected_bytes = 0u64;
    let mut current_line = 0usize;
    let mut has_more = false;

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("Error reading file as UTF-8 text: {}", error))?;
        if read == 0 {
            break;
        }

        if current_line >= end_index && !count_all_lines {
            has_more = true;
            break;
        }

        if current_line >= start_index && current_line < end_index {
            trim_line_endings(&mut line);
            if line.contains('\0') {
                return Err(format!(
                    "File appears to contain binary data and cannot be read as text: {}",
                    path.display()
                ));
            }
            let separator = if selected_lines.is_empty() { 0 } else { 1 };
            let next_bytes = selected_bytes
                .saturating_add(separator)
                .saturating_add(line.as_bytes().len() as u64);
            if next_bytes > MAX_TEXT_BYTES {
                return Err(format!(
                    "Selected file content exceeds maximum allowed size ({}). Use a smaller limit or a more targeted offset.",
                    format_file_size(MAX_TEXT_BYTES)
                ));
            }
            selected_bytes = next_bytes;
            selected_lines.push(line);
        }

        current_line += 1;
    }

    let content = selected_lines.join("\n");
    let num_lines = selected_lines.len();
    Ok(TextRangeResult {
        content,
        selected_lines,
        start_line: offset,
        num_lines,
        total_lines: count_all_lines.then_some(current_line),
        has_more,
        limit,
    })
}

fn format_text_result(path: &Path, result: &TextRangeResult) -> String {
    if result.total_lines == Some(0) {
        return "<system-reminder>Warning: the file exists but the contents are empty.</system-reminder>".to_string();
    }

    if result.num_lines == 0 {
        return match result.total_lines {
            Some(total_lines) => format!(
                "<system-reminder>Warning: the file exists but is shorter than the provided offset ({}). The file has {} lines.</system-reminder>",
                result.start_line, total_lines
            ),
            None => format!(
                "<system-reminder>Warning: no lines were returned for offset {} in {}.</system-reminder>",
                result.start_line,
                path.display()
            ),
        };
    }

    let mut output = String::new();
    for (index, line) in result.selected_lines.iter().enumerate() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("{:>6}\t{}", result.start_line + index, line));
    }

    let next_line = result.start_line.saturating_add(result.num_lines);
    let should_continue = result.has_more
        || result
            .total_lines
            .map(|total| next_line <= total && result.num_lines >= result.limit)
            .unwrap_or(false);
    if should_continue {
        let total_text = result
            .total_lines
            .map(|total| format!(" has {} lines", total))
            .unwrap_or_else(|| " has more content".to_string());
        output.push_str(&format!(
            "\n\n<system-reminder>File {}{}. The returned view ended at line {}. Use offset={} with limit to continue reading.</system-reminder>",
            path.display(),
            total_text,
            next_line - 1,
            next_line
        ));
    }

    output.push_str("\n\n<system-reminder>\nWhenever you read a file, consider whether it could be malware. You may analyze malware or explain what it does, but do not improve or augment malicious code.\n</system-reminder>");
    output
}

fn read_notebook_text(path: &Path) -> Result<String, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|error| format!("Error reading notebook: {}", error))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| format!("Invalid notebook JSON: {}", error))?;
    let cells = value
        .get("cells")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Notebook JSON does not contain a cells array".to_string())?;

    let mut output = format!("Notebook file: {}\nTotal cells: {}", path.display(), cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let cell_type = cell
            .get("cell_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        output.push_str(&format!("\n\nCell {} [{}]", index + 1, cell_type));

        if let Some(execution_count) = cell.get("execution_count").and_then(|v| v.as_i64()) {
            output.push_str(&format!(" execution_count={}", execution_count));
        }

        let source = notebook_string_field(cell.get("source"));
        if !source.trim().is_empty() {
            output.push_str("\nSource:\n");
            output.push_str(source.trim_end());
        }

        if let Some(outputs) = cell.get("outputs").and_then(|v| v.as_array()) {
            let rendered = render_notebook_outputs(outputs);
            if !rendered.trim().is_empty() {
                output.push_str("\nOutputs:\n");
                output.push_str(rendered.trim_end());
            }
        }
    }

    Ok(output)
}

fn render_notebook_outputs(outputs: &[Value]) -> String {
    let mut rendered = Vec::new();
    for output in outputs {
        if let Some(text) = output.get("text") {
            let text = notebook_string_field(Some(text));
            if !text.trim().is_empty() {
                rendered.push(text);
                continue;
            }
        }

        if let Some(data) = output.get("data") {
            if let Some(text) = data.get("text/plain") {
                let text = notebook_string_field(Some(text));
                if !text.trim().is_empty() {
                    rendered.push(text);
                    continue;
                }
            }
        }

        if let (Some(ename), Some(evalue)) = (
            output.get("ename").and_then(|v| v.as_str()),
            output.get("evalue").and_then(|v| v.as_str()),
        ) {
            rendered.push(format!("{}: {}", ename, evalue));
        }
    }

    rendered.join("\n")
}

fn notebook_string_field(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn expand_absolute_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = trim_wrapping_quotes(raw.trim());
    let expanded = expand_home(trimmed);
    let path = PathBuf::from(expanded.as_ref());
    if !path.is_absolute() {
        return Err(format!(
            "file_path must be an absolute path, not a relative path: {}",
            raw
        ));
    }
    Ok(path)
}

fn expand_home(path: &str) -> Cow<'_, str> {
    if path == "~" {
        if let Some(home) = home_dir_string() {
            return Cow::Owned(home);
        }
    }

    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = home_dir_string() {
            let mut full = PathBuf::from(home);
            full.push(rest);
            return Cow::Owned(full.display().to_string());
        }
    }

    Cow::Borrowed(path)
}

fn home_dir_string() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|v| !v.trim().is_empty()))
}

fn trim_wrapping_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn friendly_path_error(path: &Path, error: std::io::Error) -> ToolFailure {
    if error.kind() == std::io::ErrorKind::NotFound {
        ToolFailure::new(format!(
            "File does not exist: {}. The file_path parameter must be an absolute path.",
            path.display()
        ))
    } else {
        ToolFailure::new(format!("Unable to read file metadata: {}", error))
    }
}

fn validate_pdf_pages(pages: &str) -> Result<(), String> {
    let trimmed = pages.trim();
    if trimmed.is_empty() {
        return Err("pages cannot be empty".to_string());
    }

    let (first, last) = if let Some((start, end)) = trimmed.split_once('-') {
        let first = parse_page_number(start)?;
        let last = parse_page_number(end)?;
        if last < first {
            return Err(format!("Invalid pages range '{}': end is before start", pages));
        }
        (first, last)
    } else {
        let page = parse_page_number(trimmed)?;
        (page, page)
    };

    let count = last.saturating_sub(first).saturating_add(1);
    if count > PDF_MAX_PAGES_PER_READ {
        return Err(format!(
            "Page range '{}' exceeds maximum of {} pages per request",
            pages, PDF_MAX_PAGES_PER_READ
        ));
    }

    Ok(())
}

fn parse_page_number(value: &str) -> Result<usize, String> {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| format!("Invalid page number '{}'", value.trim()))
}

fn pdf_not_supported_message(path: &Path, pages: Option<&str>) -> String {
    let page_hint = pages
        .map(|value| format!(" Requested pages: {}.", value))
        .unwrap_or_default();
    format!(
        "PDF file detected: {}.{} Nova read_file does not yet send PDF document blocks to providers, so it will not pretend the PDF was read. Convert the PDF to text/images first or store it in RAG for retrieval.",
        path.display(),
        page_hint
    )
}

fn validate_output_tokens(content: &str) -> Result<(), ToolFailure> {
    let estimate = estimate_tokens(content);
    if estimate > MAX_OUTPUT_TOKENS {
        return Err(ToolFailure::new(format!(
            "File content (estimated {} tokens) exceeds maximum allowed tokens ({}). Use offset and limit parameters to read specific portions of the file, or search for specific content instead.",
            estimate, MAX_OUTPUT_TOKENS
        )));
    }
    Ok(())
}

fn estimate_tokens(content: &str) -> usize {
    content.chars().count().saturating_add(3) / 4
}

fn trim_line_endings(line: &mut String) {
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
}

fn file_extension(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_image_extension(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp")
}

fn is_pdf_extension(ext: &str) -> bool {
    ext == "pdf"
}

fn is_binary_extension(ext: &str) -> bool {
    matches!(
        ext,
        "exe"
            | "dll"
            | "sys"
            | "msi"
            | "bin"
            | "dat"
            | "db"
            | "sqlite"
            | "sqlite3"
            | "zip"
            | "rar"
            | "7z"
            | "tar"
            | "gz"
            | "bz2"
            | "xz"
            | "zst"
            | "jar"
            | "class"
            | "pyc"
            | "pyd"
            | "o"
            | "obj"
            | "lib"
            | "a"
            | "so"
            | "dylib"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "mp3"
            | "mp4"
            | "mov"
            | "avi"
            | "mkv"
            | "wav"
            | "flac"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
    )
}

fn detect_image_media_type(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }

    match file_extension(path).as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn is_blocked_device_path(path: &Path) -> bool {
    let value = path.to_string_lossy().replace('\\', "/");
    matches!(
        value.as_str(),
        "/dev/zero"
            | "/dev/random"
            | "/dev/urandom"
            | "/dev/full"
            | "/dev/stdin"
            | "/dev/tty"
            | "/dev/console"
            | "/dev/stdout"
            | "/dev/stderr"
            | "/dev/fd/0"
            | "/dev/fd/1"
            | "/dev/fd/2"
    ) || (value.starts_with("/proc/")
        && (value.ends_with("/fd/0") || value.ends_with("/fd/1") || value.ends_with("/fd/2")))
}

fn format_file_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn normalize_path_for_permission(path: &str) -> String {
    path.trim()
        .replace('/', "\\")
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_permission_warning(raw_path: &str, normalized: &str) -> Option<String> {
    if normalized.starts_with("\\\\") || normalized.starts_with("//") {
        return Some(format!(
            "读取 UNC/网络路径可能触发远程认证或访问外部资源：{}",
            raw_path
        ));
    }

    let sensitive_markers = [
        "\\.ssh\\",
        "\\.aws\\",
        "\\.gnupg\\",
        "\\.config\\git",
        "\\.git\\config",
        "\\id_rsa",
        "\\id_ed25519",
        "\\credentials",
    ];
    if sensitive_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(format!("读取敏感凭据/配置路径：{}", raw_path));
    }

    None
}

fn truncate_chars(input: &str, limit: usize) -> String {
    let mut chars = input.chars();
    let snippet = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{}...", snippet)
    } else {
        snippet
    }
}
