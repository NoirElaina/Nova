use crate::llm::tools::{sync_tool, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub(crate) fn registration() -> ToolRegistration {
    sync_tool(tool, execute, true)
}

pub fn tool() -> Tool {
    Tool {
        name: "glob_search".into(),
        description: "Search files by wildcard pattern (supports * and ?).".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "root": { "type": "string", "description": "Root directory to search" },
                "pattern": { "type": "string", "description": "Wildcard pattern against relative path" },
                "max_results": { "type": "integer", "description": "Maximum number of matches" }
            },
            "required": ["root", "pattern"]
        }),
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();

    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut match_j) = (None, 0usize);

    while j < t.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            match_j = j;
        } else if let Some(star_idx) = star {
            i = star_idx + 1;
            match_j += 1;
            j = match_j;
        } else {
            return false;
        }
    }

    while i < p.len() && p[i] == b'*' {
        i += 1;
    }

    i == p.len()
}

fn walk(root: &Path, current: &Path, pattern: &str, out: &mut Vec<String>, max: usize) {
    if out.len() >= max {
        return;
    }

    let entries = match fs::read_dir(current) {
        Ok(v) => v,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if out.len() >= max {
            break;
        }

        let p = entry.path();
        if p.is_dir() {
            walk(root, &p, pattern, out, max);
            continue;
        }

        if let Ok(rel) = p.strip_prefix(root) {
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if wildcard_match(pattern, &rel_s) {
                out.push(p.display().to_string());
            }
        }
    }
}

pub fn execute(input: Value) -> String {
    let root = match input.get("root").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v,
        _ => return "Error: Missing 'root' argument".into(),
    };

    let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return "Error: Missing 'pattern' argument".into(),
    };

    let max_results = input
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(200)
        .max(1)
        .min(2000);

    let root_path = Path::new(root);
    if !root_path.exists() {
        return format!("Error: Root path does not exist: {}", root);
    }
    if !root_path.is_dir() {
        return format!("Error: Root path is not a directory: {}", root);
    }

    let mut out = Vec::new();
    walk(root_path, root_path, pattern, &mut out, max_results);

    if out.is_empty() {
        "No files matched the pattern".into()
    } else {
        out.join("\n")
    }
}
