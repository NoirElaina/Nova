use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

// 把 Skill 工具的 async 执行逻辑包装成统一 future。
// `app` 只用来定位 skills 根目录，`input` 里带 action/skill/args。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 Skill 工具的注册信息。
// 它只读本地 skill 文件，不直接改写状态，所以标成只读工具。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
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

// 返回模型可见的 Skill 工具元数据。
// 模型先 `action=list` 发现技能，再 `action=run` 加载某个技能说明。
pub fn tool() -> Tool {
    Tool {
        name: "Skill".into(),
        description: "Execute a skill by name. Use action=list to discover available skills, then action=run to load one.".into(),
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

// 把任意长字符串截成最多 `max_chars` 个字符，避免把大段 skill 内容原样塞回模型。
fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect::<String>()
}

// 把 skill 名字归一化成便于比较的形式。
// 当前只做 trim + 小写，用来支持大小写不敏感匹配。
fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

// 去掉 SKILL.md 头部的 frontmatter，只保留正文说明。
// `raw` 是文件原始文本，返回值是提供给模型阅读的纯说明内容。
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

// 从 frontmatter 里读取指定 key 的值。
// `key` 常用来提取 `name` 或 `description` 这些元信息。
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

// 决定一个 skill 最终显示给模型的名字。
// 优先读 frontmatter 的 `name`，否则退回到 skill 目录名。
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

// 决定一个 skill 的简短描述。
// 优先读 frontmatter 的 `description`，否则取正文里第一条非空文本。
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

// 递归收集 root 目录下所有名为 `SKILL.md` 的文件。
// `out` 是收集结果的可变数组，函数本身不做去重。
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

// 返回 skills 根目录 `<app_data_dir>/skills`。
// Skill 工具所有磁盘扫描都从这个目录开始。
fn skills_root_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("skills"))
        .map_err(|e| format!("Failed to resolve app_data_dir for skills: {}", e))
}

// 从本地 skills 目录加载全部 skill 元数据和正文内容。
// `skill_files` 是扫到的 SKILL.md 文件列表，`out` 是最终返回的去重结果。
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

// 返回给其他模块使用的轻量 skill 摘要。
// 这里只暴露 name/description/path，不把正文 content 一起带出去。
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

// 把 skills 列表转成模型更容易消费的 JSON 数组。
fn list_skills(skills: &[SkillEntry]) -> ToolOutcome {
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

    ToolOutcome::json(json!({
        "ok": true,
        "skills": items
    }))
}

// 收集某个 skill 所在目录下的附属文件列表。
// 返回 `(相对路径, 绝对路径)`，方便模型知道还可以继续读取哪些文件。
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

// 递归收集目录里的普通文件，并记录相对路径。
// `root` 用来算相对路径，`current` 是当前递归目录，`out` 是累计结果。
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

// 按名字挑出一个 skill，并把正文和附属文件说明打包成返回文本。
// `skill_name` 是模型请求的技能名，`args` 是附加给 skill 的可选参数提示。
fn run_skill(
    skills: &[SkillEntry],
    skill_name: &str,
    args: Option<&str>,
) -> Result<ToolOutcome, ToolFailure> {
    // skill_name_norm: 归一化后的名字，用于大小写不敏感查找。
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
        return Err(ToolFailure::new(format!(
            "Skill '{}' not found. Use action=list to see available skills.",
            skill_name
        )));
    };

    let skill_dir = skill
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    // sibling_files: skill 目录下可继续读取的辅文件清单，例如 scripts/、references/ 等。
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

    Ok(ToolOutcome::json(json!({
        "ok": true,
        "content": out
    })))
}

// 根据 action 执行 `list` 或 `run`。
// `skills` 是当前磁盘上实际存在的技能集合，`skill` 是模型请求运行的目标技能名。
async fn execute_with_app(app: &AppHandle, input: Value) -> Result<ToolOutcome, ToolFailure> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("run")
        .trim()
        .to_ascii_lowercase();

    let skills = match load_skills(app) {
        Ok(skills) => skills,
        Err(e) => return Err(ToolFailure::new(e)),
    };

    if action == "list" {
        return Ok(list_skills(&skills));
    }

    let skill = match input.get("skill").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => return Err(ToolFailure::invalid_input("Missing 'skill' argument for action=run")),
    };

    let args = input.get("args").and_then(|v| v.as_str());
    run_skill(&skills, skill, args)
}
