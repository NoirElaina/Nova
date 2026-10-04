use serde_json::{json, Value};
use crate::agent::tools::verifier::CodeVerifier;
use crate::llm::utils::file_io::{read_file_meta, resolve_tool_path, write_file_with_meta};

#[derive(Debug)]
pub struct EditResult {
    pub file_path: String,
    pub occurrences_replaced: usize,
}

/// 工业级精确块匹配文件修改引擎（Exact Chunk Replacer）
/// 严格遵循“精确匹配 + 唯一性校验 + 语法自愈验证”，杜绝模糊猜测导致的乱改误改。
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
    if count == 0 {
        return Err(format!(
            "Could not find exact match for old_string in {}. Please re-read the file and provide exact lines with context.",
            file_path
        ));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "Found {} occurrences of old_string in {}. Please include more surrounding context lines to make the match unique, or specify replace_all=true.",
            count, file_path
        ));
    }

    let modified = if replace_all {
        original.replace(&old_normalized, &new_normalized)
    } else {
        original.replacen(&old_normalized, &new_normalized, 1)
    };

    // 静态自愈语法验证：写入前预检
    CodeVerifier::verify_file_content(&target, &modified).await?;

    // 原子安全写入磁盘
    write_file_with_meta(&target, &modified, &meta)
        .map_err(|e| format!("Failed to write file {}: {}", file_path, e))?;

    Ok(json!({
        "ok": true,
        "file_path": file_path,
        "occurrences_replaced": if replace_all { count } else { 1 }
    }))
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct EditOperation {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
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
        if count == 0 {
            return Err(format!(
                "edits[{}]: Could not find exact match for old_string in {}. Please re-read the file and provide exact lines with context.",
                idx, file_path
            ));
        }
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
    }

    // 语法自愈验证：全部 edits 批处理完成后进行 AST 级校验
    CodeVerifier::verify_file_content(&target, &content).await?;

    // 全部成功后原子安全写回
    write_file_with_meta(&target, &content, &meta)
        .map_err(|e| format!("Failed to write file {}: {}", file_path, e))?;

    Ok(json!({
        "ok": true,
        "file_path": file_path,
        "edits_applied": edits.len(),
        "occurrences_replaced": applied_count
    }))
}
