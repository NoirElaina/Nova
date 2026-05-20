use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::llm::commands::types::GlobalMemoryEntry;

const MEMORY_ROOT_DIR: &str = "memory";
const MEMORY_INDEX_FILE: &str = "MEMORY.md";
const MEMORY_KINDS: [&str; 3] = ["preference", "fact", "rule"];

#[derive(Debug, Clone)]
struct MemoryFileRecord {
    entry: GlobalMemoryEntry,
    file_path: PathBuf,
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn memory_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(MEMORY_ROOT_DIR);
    ensure_memory_layout(&root)?;
    Ok(root)
}

fn ensure_memory_layout(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    for kind in MEMORY_KINDS {
        fs::create_dir_all(root.join(kind)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn normalize_kind(raw: Option<&str>) -> String {
    match raw.unwrap_or("fact").trim().to_ascii_lowercase().as_str() {
        "preference" => "preference".to_string(),
        "rule" => "rule".to_string(),
        _ => "fact".to_string(),
    }
}

fn normalize_source(raw: Option<&str>) -> String {
    let normalized = raw.unwrap_or("assistant").trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "assistant".to_string()
    } else {
        normalized
    }
}

fn normalize_content(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_for_match(raw: &str) -> String {
    let normalized = normalize_content(raw);
    let mut out = String::new();
    let mut prev_space = false;
    for ch in normalized.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }

        if ch.is_ascii_punctuation() || ch.is_ascii_control() {
            continue;
        }

        if ch.is_ascii() {
            out.push(ch.to_ascii_lowercase());
        } else if !ch.is_control() && !ch.is_ascii_punctuation() {
            out.push(ch);
        }
        prev_space = false;
    }
    out.trim().to_string()
}

fn strip_memory_prefixes(raw: &str) -> String {
    let mut text = normalize_for_match(raw);
    let prefixes = [
        "记住 ",
        "以后 ",
        "以后请 ",
        "请你 ",
        "请 ",
        "默认 ",
        "优先 ",
        "我更喜欢 ",
        "我希望 ",
        "from now on ",
        "please ",
        "default to ",
        "prefer ",
        "remember ",
    ];
    loop {
        let mut changed = false;
        for prefix in prefixes {
            if let Some(rest) = text.strip_prefix(prefix) {
                let trimmed = rest.trim().to_string();
                if !trimmed.is_empty() {
                    text = trimmed;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    text
}

fn semantic_memory_key(_kind: &str, content: &str) -> String {
    let stripped = strip_memory_prefixes(content);
    if stripped.is_empty() {
        normalize_for_match(content)
    } else {
        stripped
    }
}

fn conflict_dimensions(kind: &str, content: &str) -> HashSet<String> {
    if !matches!(kind, "preference" | "rule") {
        return HashSet::new();
    }

    let normalized = normalize_for_match(content);
    let mut dimensions = HashSet::new();

    let register = |dimensions: &mut HashSet<String>, dimension: &str, tokens: &[&str]| {
        if tokens.iter().any(|token| normalized.contains(token)) {
            dimensions.insert(dimension.to_string());
        }
    };

    register(
        &mut dimensions,
        "language",
        &["中文", "汉语", "chinese", "简体中文", "英文", "english"],
    );
    register(
        &mut dimensions,
        "verbosity",
        &[
            "简洁",
            "简短",
            "concise",
            "brief",
            "short",
            "详细",
            "detailed",
            "step by step",
        ],
    );
    register(
        &mut dimensions,
        "fallback_policy",
        &[
            "不要兜底",
            "不要 fallback",
            "不保留兼容",
            "remove fallback",
            "no fallback",
            "no compatibility shim",
        ],
    );
    register(
        &mut dimensions,
        "tool_behavior",
        &[
            "直接改",
            "直接修改",
            "不要只分析",
            "do not just analyze",
            "edit directly",
        ],
    );

    if let Some(head) = strip_memory_prefixes(content)
        .split(['，', '。', ',', '.', ';', '；', ':', '：', '\n'])
        .next()
        .map(str::trim)
        .filter(|segment| segment.chars().count() >= 6)
    {
        dimensions.insert(format!("statement:{}", normalize_for_match(head)));
    }

    dimensions
}

fn purge_conflicting_records(
    records: &mut Vec<MemoryFileRecord>,
    preserved_id: i64,
    kind: &str,
    content: &str,
) -> Result<(), String> {
    let dimensions = conflict_dimensions(kind, content);
    if dimensions.is_empty() {
        return Ok(());
    }

    let mut removed = Vec::new();
    records.retain(|record| {
        if record.entry.id == preserved_id
            || !matches!(record.entry.kind.as_str(), "preference" | "rule")
        {
            return true;
        }

        let record_dimensions = conflict_dimensions(&record.entry.kind, &record.entry.content);
        let conflicted = !record_dimensions.is_disjoint(&dimensions);
        if conflicted {
            removed.push(record.file_path.clone());
        }
        !conflicted
    });

    for path in removed {
        if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

fn stable_memory_id(content: &str) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    (hasher.finish() & 0x7fff_ffff_ffff_ffff) as i64
}

fn slugify(content: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in content.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            slug.push(lower);
            prev_dash = false;
            continue;
        }

        if ch.is_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
            continue;
        }

        if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "memory".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn build_memory_path(root: &Path, entry: &GlobalMemoryEntry) -> PathBuf {
    let file_name = format!("{:016x}-{}.md", entry.id as u64, slugify(&entry.content));
    root.join(&entry.kind).join(file_name)
}

fn truncate_line(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    for ch in trimmed.chars().take(max_chars) {
        out.push(ch);
    }
    if trimmed.chars().count() > max_chars {
        format!("{}...", out)
    } else {
        out
    }
}

fn parse_frontmatter(document: &str) -> Option<(Vec<(String, String)>, String)> {
    let rest = document.strip_prefix("---\n")?;
    let boundary = rest.find("\n---\n")?;
    let frontmatter = &rest[..boundary];
    let body = rest[boundary + 5..].trim().to_string();
    let fields = frontmatter
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    Some((fields, body))
}

fn record_from_file(path: PathBuf) -> Result<MemoryFileRecord, String> {
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let (fields, body) = parse_frontmatter(&raw)
        .ok_or_else(|| format!("Invalid memory file format: {}", path.display()))?;

    let lookup = |key: &str| -> Option<String> {
        fields
            .iter()
            .find_map(|(field_key, field_value)| (field_key == key).then(|| field_value.clone()))
    };

    let content = normalize_content(&body);
    if content.is_empty() {
        return Err(format!("Memory file is empty: {}", path.display()));
    }

    let id = lookup("id")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_else(|| stable_memory_id(&content));
    let created_at = lookup("created_at")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let updated_at = lookup("updated_at")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(created_at);
    let hits = lookup("hits")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);

    Ok(MemoryFileRecord {
        entry: GlobalMemoryEntry {
            id,
            content,
            kind: normalize_kind(lookup("kind").as_deref()),
            source: normalize_source(lookup("source").as_deref()),
            hits,
            created_at,
            updated_at,
        },
        file_path: path,
    })
}

fn write_record(root: &Path, record: &MemoryFileRecord) -> Result<PathBuf, String> {
    ensure_memory_layout(root)?;
    let target_path = build_memory_path(root, &record.entry);
    if record.file_path != target_path && record.file_path.exists() {
        fs::remove_file(&record.file_path).map_err(|e| e.to_string())?;
    }

    let document = format!(
        "---\nid: {}\nkind: {}\nsource: {}\nhits: {}\ncreated_at: {}\nupdated_at: {}\n---\n{}\n",
        record.entry.id,
        record.entry.kind,
        record.entry.source,
        record.entry.hits,
        record.entry.created_at,
        record.entry.updated_at,
        record.entry.content
    );

    fs::write(&target_path, document).map_err(|e| e.to_string())?;
    Ok(target_path)
}

fn rebuild_index(root: &Path, records: &[MemoryFileRecord]) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sorted.sort_by(|a, b| {
        b.entry
            .updated_at
            .cmp(&a.entry.updated_at)
            .then_with(|| b.entry.id.cmp(&a.entry.id))
    });

    let mut lines = vec![
        "# Memory Index".to_string(),
        "".to_string(),
        "Persistent memories available across chat sessions.".to_string(),
        "".to_string(),
    ];

    for record in sorted {
        let relative = record
            .file_path
            .strip_prefix(root)
            .unwrap_or(record.file_path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        lines.push(format!(
            "- [{}]({}) — {} | source={} | hits={}",
            truncate_line(&record.entry.content, 72),
            relative,
            record.entry.kind,
            record.entry.source,
            record.entry.hits
        ));
    }

    fs::write(root.join(MEMORY_INDEX_FILE), lines.join("\n")).map_err(|e| e.to_string())
}

fn read_all_records(root: &Path) -> Result<Vec<MemoryFileRecord>, String> {
    ensure_memory_layout(root)?;
    let mut records = Vec::new();

    for kind in MEMORY_KINDS {
        let dir = root.join(kind);
        if !dir.exists() {
            continue;
        }

        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            records.push(record_from_file(path)?);
        }
    }

    Ok(records)
}

fn tokenize_query(text: &str) -> HashSet<String> {
    let normalized = text.trim().to_ascii_lowercase();
    let mut tokens = HashSet::new();

    for token in normalized
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() >= 3)
    {
        tokens.insert(token.to_string());
    }

    let compact_chars = normalized
        .chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_ascii_punctuation())
        .collect::<Vec<_>>();
    for window in compact_chars.windows(2) {
        tokens.insert(window.iter().collect::<String>());
    }
    for window in compact_chars.windows(3) {
        tokens.insert(window.iter().collect::<String>());
    }

    tokens
}

fn score_record(query_tokens: &HashSet<String>, record: &MemoryFileRecord) -> i64 {
    if query_tokens.is_empty() {
        return match record.entry.kind.as_str() {
            "rule" => 40,
            "preference" => 30,
            _ => 0,
        };
    }

    let haystack = format!(
        "{} {} {}",
        record.entry.kind,
        record.entry.source,
        record.entry.content.to_ascii_lowercase()
    );
    let mut score = match record.entry.kind.as_str() {
        "rule" => 24,
        "preference" => 18,
        _ => 0,
    };

    for token in query_tokens {
        if haystack.contains(token) {
            score += if token.chars().count() >= 3 { 10 } else { 4 };
        }
    }

    score
}

fn select_relevant_records(
    records: &[MemoryFileRecord],
    query: Option<&str>,
    limit: usize,
) -> Vec<MemoryFileRecord> {
    let query_tokens = query.map(tokenize_query).unwrap_or_default();

    let mut always = records
        .iter()
        .filter(|record| matches!(record.entry.kind.as_str(), "preference" | "rule"))
        .cloned()
        .collect::<Vec<_>>();
    always.sort_by(|a, b| {
        score_record(&query_tokens, b)
            .cmp(&score_record(&query_tokens, a))
            .then_with(|| b.entry.updated_at.cmp(&a.entry.updated_at))
    });
    always.truncate(limit.min(4));

    let taken_ids = always
        .iter()
        .map(|record| record.entry.id)
        .collect::<HashSet<_>>();
    let mut facts = records
        .iter()
        .filter(|record| !taken_ids.contains(&record.entry.id))
        .cloned()
        .collect::<Vec<_>>();
    facts.sort_by(|a, b| {
        score_record(&query_tokens, b)
            .cmp(&score_record(&query_tokens, a))
            .then_with(|| b.entry.updated_at.cmp(&a.entry.updated_at))
    });

    let remaining = limit.saturating_sub(always.len());
    let mut selected = always;
    selected.extend(
        facts
            .into_iter()
            .filter(|record| !query_tokens.is_empty() || record.entry.kind != "fact")
            .take(remaining),
    );

    if selected.is_empty() {
        let mut fallback = records.to_vec();
        fallback.sort_by(|a, b| {
            b.entry
                .updated_at
                .cmp(&a.entry.updated_at)
                .then_with(|| b.entry.id.cmp(&a.entry.id))
        });
        fallback.truncate(limit);
        return fallback;
    }

    selected
}

fn bump_hits(root: &Path, records: &[MemoryFileRecord]) -> Result<(), String> {
    for record in records {
        let mut next = record.clone();
        next.entry.hits += 1;
        next.file_path = write_record(root, &next)?;
    }
    Ok(())
}

pub async fn list_global_memory(
    app: &AppHandle,
    limit: Option<i64>,
) -> Result<Vec<GlobalMemoryEntry>, String> {
    let root = memory_root(app)?;
    let mut records = read_all_records(&root)?;
    records.sort_by(|a, b| {
        b.entry
            .updated_at
            .cmp(&a.entry.updated_at)
            .then_with(|| b.entry.id.cmp(&a.entry.id))
    });

    Ok(records
        .into_iter()
        .take(limit.unwrap_or(12).clamp(1, 100) as usize)
        .map(|record| record.entry)
        .collect())
}

pub async fn upsert_global_memory(
    app: &AppHandle,
    content: &str,
    kind: Option<&str>,
    source: Option<&str>,
) -> Result<GlobalMemoryEntry, String> {
    let normalized_content = normalize_content(content);
    if normalized_content.is_empty() {
        return Err("global memory content is empty".to_string());
    }

    let root = memory_root(app)?;
    let kind = normalize_kind(kind);
    let source = normalize_source(source);
    let now = now_timestamp();
    let mut records = read_all_records(&root)?;
    let semantic_key = semantic_memory_key(&kind, &normalized_content);

    if let Some(index) = records.iter().position(|record| {
        semantic_memory_key(&record.entry.kind, &record.entry.content) == semantic_key
    }) {
        let entry = {
            let existing = &mut records[index];
            existing.entry.content = normalized_content;
            existing.entry.kind = kind;
            existing.entry.source = source;
            existing.entry.hits += 1;
            existing.entry.updated_at = now;
            existing.file_path = write_record(&root, existing)?;
            existing.entry.clone()
        };
        purge_conflicting_records(&mut records, entry.id, &entry.kind, &entry.content)?;
        rebuild_index(&root, &records)?;
        return Ok(entry);
    }

    let entry = GlobalMemoryEntry {
        id: stable_memory_id(&normalized_content),
        content: normalized_content,
        kind,
        source,
        hits: 1,
        created_at: now,
        updated_at: now,
    };
    let record = MemoryFileRecord {
        file_path: build_memory_path(&root, &entry),
        entry: entry.clone(),
    };
    let mut record = record;
    record.file_path = write_record(&root, &record)?;
    records.push(record);
    purge_conflicting_records(&mut records, entry.id, &entry.kind, &entry.content)?;
    rebuild_index(&root, &records)?;
    Ok(entry)
}

pub async fn delete_global_memory(app: &AppHandle, id: i64) -> Result<bool, String> {
    let root = memory_root(app)?;
    let mut records = read_all_records(&root)?;
    let Some(index) = records.iter().position(|record| record.entry.id == id) else {
        return Ok(false);
    };

    let removed = records.remove(index);
    if removed.file_path.exists() {
        fs::remove_file(&removed.file_path).map_err(|e| e.to_string())?;
    }
    rebuild_index(&root, &records)?;
    Ok(true)
}

pub async fn clear_global_memory(app: &AppHandle) -> Result<i64, String> {
    let root = memory_root(app)?;
    let records = read_all_records(&root)?;
    let count = records.len() as i64;

    if root.exists() {
        fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    }
    ensure_memory_layout(&root)?;
    rebuild_index(&root, &[])?;
    Ok(count)
}

pub async fn relevant_global_memory(
    app: &AppHandle,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<GlobalMemoryEntry>, String> {
    let root = memory_root(app)?;
    let records = read_all_records(&root)?;
    let selected = select_relevant_records(&records, query, limit);
    bump_hits(&root, &selected)?;
    let updated_records = read_all_records(&root)?;
    let selected_ids = selected
        .iter()
        .map(|record| record.entry.id)
        .collect::<HashSet<_>>();

    Ok(updated_records
        .into_iter()
        .filter(|record| selected_ids.contains(&record.entry.id))
        .map(|record| record.entry)
        .collect())
}
