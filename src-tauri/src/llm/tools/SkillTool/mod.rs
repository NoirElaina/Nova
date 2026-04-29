use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, true)
}

#[derive(Debug, Clone)]
struct SkillEntry {
    name: String,
    description: String,
    path: PathBuf,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: String,
}

pub fn tool() -> Tool {
    Tool {
        name: "Skill".into(),
        description: "Execute a skill by name (Claude-compatible Skill tool). Use action=list to discover available skills, then action=run to load one.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "run"],
                    "description": "list: return available skills; run: load a specific skill"
                },
                "skill": {
                    "type": "string",
                    "description": "Skill name to run, e.g. 'claude-source-migration-check'"
                },
                "args": {
                    "type": "string",
                    "description": "Optional skill arguments"
                }
            }
        }),
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect::<String>()
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn strip_frontmatter(raw: &str) -> String {
    let mut lines = raw.lines();
    if lines.next() != Some("---") {
        return raw.trim().to_string();
    }

    let mut body = Vec::new();
    let mut in_frontmatter = true;
    for line in lines {
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        body.push(line);
    }

    body.join("\n").trim().to_string()
}

fn parse_frontmatter_value(raw: &str, key: &str) -> Option<String> {
    let mut lines = raw.lines();
    if lines.next() != Some("---") {
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim() == key {
                return Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
            }
        }
    }

    None
}

fn pick_skill_name(path: &Path, raw: &str) -> String {
    parse_frontmatter_value(raw, "name")
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown-skill".to_string())
}

fn pick_skill_description(raw: &str) -> String {
    if let Some(desc) = parse_frontmatter_value(raw, "description") {
        if !desc.trim().is_empty() {
            return truncate_chars(desc.trim(), 180);
        }
    }

    let body = strip_frontmatter(raw);
    for line in body.lines() {
        let t = line.trim();
        if !t.is_empty() {
            return truncate_chars(t, 180);
        }
    }

    "(no description)".to_string()
}

fn collect_skill_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(&path, out);
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if file_name.eq_ignore_ascii_case("SKILL.md") {
            out.push(path);
        }
    }
}

fn skills_root_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("skills"))
        .map_err(|e| format!("Failed to resolve app_data_dir for skills: {}", e))
}

fn load_skills(app: &AppHandle) -> Result<Vec<SkillEntry>, String> {
    let mut skill_files = Vec::new();
    let skills_root = skills_root_dir(app)?;

    if skills_root.exists() {
        collect_skill_files(&skills_root, &mut skill_files);
    }

    let mut out = Vec::new();
    for path in skill_files {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };

        let name = pick_skill_name(&path, &raw);
        let description = pick_skill_description(&raw);
        let content = strip_frontmatter(&raw);

        out.push(SkillEntry {
            name,
            description,
            path,
            content,
        });
    }

    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| normalize_name(&a.name) == normalize_name(&b.name));
    Ok(out)
}

pub fn list_skill_summaries_with_app(app: &AppHandle) -> Result<Vec<SkillSummary>, String> {
    Ok(load_skills(app)?
        .into_iter()
        .map(|s| SkillSummary {
            name: s.name,
            description: s.description,
            path: s.path.display().to_string(),
        })
        .collect())
}

fn list_skills(skills: &[SkillEntry]) -> String {
    let items = skills
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "path": s.path.display().to_string()
            })
        })
        .collect::<Vec<_>>();

    json!({
        "ok": true,
        "skills": items
    })
    .to_string()
}

fn collect_skill_sibling_files(skill_md_path: &Path) -> Vec<(String, PathBuf)> {
    let Some(skill_dir) = skill_md_path.parent() else {
        return vec![];
    };

    let mut result = Vec::new();
    collect_dir_files_relative(skill_dir, skill_dir, &mut result);
    // 排除 SKILL.md 自身和许可文件
    result
        .into_iter()
        .filter(|(rel, _)| {
            let lower = rel.to_ascii_lowercase();
            lower != "skill.md" && !lower.ends_with("license.txt")
        })
        .collect()
}

fn collect_dir_files_relative(root: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(current) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_files_relative(root, &path, out);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if !rel.is_empty() {
                out.push((rel, path));
            }
        }
    }
}

fn run_skill(skills: &[SkillEntry], skill_name: &str, args: Option<&str>) -> String {
    let skill_name_norm = normalize_name(skill_name);

    let picked = skills
        .iter()
        .find(|s| normalize_name(&s.name) == skill_name_norm)
        .or_else(|| {
            skills
                .iter()
                .find(|s| normalize_name(&s.name).contains(&skill_name_norm))
        });

    let Some(skill) = picked else {
        return json!({
            "ok": false,
            "error": format!("Skill '{}' not found. Use action=list to see available skills.", skill_name)
        })
        .to_string();
    };

    let skill_dir = skill
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let sibling_files = collect_skill_sibling_files(&skill.path);

    let mut out = String::new();
    out.push_str(&format!("Skill: {}\n", skill.name));
    out.push_str(&format!("Description: {}\n", skill.description));
    out.push_str(&format!("Skill directory: {}\n", skill_dir));
    if !sibling_files.is_empty() {
        out.push_str("Additional skill files (use file_read with absolute path to access):\n");
        for (rel, abs) in &sibling_files {
            out.push_str(&format!("  {} -> {}\n", rel, abs.display()));
        }
    }
    if let Some(a) = args {
        if !a.trim().is_empty() {
            out.push_str(&format!("Args: {}\n", a.trim()));
        }
    }
    out.push_str("\nSkill instructions:\n");
    out.push_str(&truncate_chars(&skill.content, 20_000));

    json!({
        "ok": true,
        "content": out
    })
    .to_string()
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "skill requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("run")
        .trim()
        .to_ascii_lowercase();

    let skills = match load_skills(app) {
        Ok(skills) => skills,
        Err(e) => return json!({ "ok": false, "error": e }).to_string(),
    };

    if action == "list" {
        return list_skills(&skills);
    }

    let skill = match input.get("skill").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return json!({ "ok": false, "error": "Missing 'skill' argument for action=run" }).to_string(),
    };

    let args = input.get("args").and_then(|v| v.as_str());
    run_skill(&skills, skill, args)
}
