use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub(crate) fn trim_wrapping_quotes(value: &str) -> &str {
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

pub(crate) fn absolute_path_from_tool_arg(raw: &str, field_name: &str) -> Result<PathBuf, String> {
    let trimmed = trim_wrapping_quotes(raw.trim());
    if trimmed.is_empty() {
        return Err(format!("{} is required", field_name));
    }

    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!(
            "{} must be an absolute path; relative paths are not supported: {}",
            field_name, raw
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "{} must be a normalized absolute path without '..' components: {}",
            field_name, raw
        ));
    }

    Ok(path)
}

pub(crate) fn resolve_absolute_path_for_write(
    raw: &str,
    field_name: &str,
) -> Result<PathBuf, String> {
    let path = absolute_path_from_tool_arg(raw, field_name)?;
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|error| format!("failed to resolve path: {}", error));
    }

    let mut ancestor: &Path = &path;
    let mut missing = Vec::<OsString>::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| format!("path cannot be resolved: {}", raw))?;
        missing.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("path cannot be resolved: {}", raw))?;
    }

    let canonical_ancestor = ancestor
        .canonicalize()
        .map_err(|error| format!("failed to resolve path parent: {}", error))?;
    if !canonical_ancestor.is_dir() {
        return Err(format!("path parent is not a directory: {}", raw));
    }

    let mut resolved = canonical_ancestor;
    for part in missing.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}
