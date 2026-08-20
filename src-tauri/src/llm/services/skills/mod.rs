use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone)]
pub(crate) struct SkillEntry {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) path: PathBuf,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: String,
}

// 把任意长字符串截成最多 `max_chars` 个字符，避免把大段 skill 内容原样塞回模型。
pub(crate) fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect::<String>()
}

// 把 skill 名字归一化成便于比较的形式。
pub(crate) fn normalize_skill_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

// 去掉 SKILL.md 头部的 frontmatter，只保留正文说明。
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

// 决定一个 skill 最终显示给模型/前端的名字。
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
fn skills_root_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("skills"))
        .map_err(|e| format!("Failed to resolve app_data_dir for skills: {}", e))
}

// 从本地 skills 目录加载全部 skill 元数据和正文内容。
// 不过滤停用状态：设置页需要展示全部技能（含已停用）。
pub(crate) fn load_skills_with_app(app: &AppHandle) -> Result<Vec<SkillEntry>, String> {
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
    out.dedup_by(|a, b| normalize_skill_name(&a.name) == normalize_skill_name(&b.name));
    Ok(out)
}

// 从设置读取被停用的技能名集合（归一化小写）。
fn disabled_skill_names(app: &AppHandle) -> HashSet<String> {
    crate::command::settings::load_settings(app)
        .map(|settings| {
            settings
                .disabled_skills
                .into_iter()
                .map(|name| normalize_skill_name(&name))
                .collect()
        })
        .unwrap_or_default()
}

// 仅加载已启用的技能：系统提示词注入与 SkillTool 执行都走这里，
// 停用的技能对模型完全不可见、不可运行。
pub(crate) fn load_enabled_skills_with_app(app: &AppHandle) -> Result<Vec<SkillEntry>, String> {
    let disabled = disabled_skill_names(app);
    if disabled.is_empty() {
        return load_skills_with_app(app);
    }
    Ok(load_skills_with_app(app)?
        .into_iter()
        .filter(|s| !disabled.contains(&normalize_skill_name(&s.name)))
        .collect())
}

// 智能体私有技能：扫描 agents/<bundle_id>/skills/ 下的 SKILL.md。
// 私有技能不进全局列表、不受全局停用名单和 bundle 白名单约束——
// 它们由该智能体目录持有，挂载该智能体的会话始终可见。
pub(crate) fn load_agent_private_skills(
    app: &AppHandle,
    bundle_id: &str,
) -> Result<Vec<SkillEntry>, String> {
    let skills_dir =
        crate::llm::services::agent_bundles::agent_skills_dir(app, bundle_id)?;
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let mut skill_files = Vec::new();
    collect_skill_files(&skills_dir, &mut skill_files);

    let mut out = Vec::new();
    for path in skill_files {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        out.push(SkillEntry {
            name: pick_skill_name(&path, &raw),
            description: pick_skill_description(&raw),
            path,
            content: strip_frontmatter(&raw),
        });
    }
    Ok(out)
}

/// 合并某会话可见的技能：全局已启用技能（按 bundle 白名单过滤）+ bundle 私有技能。
/// bundle 为 None 时即全局已启用技能。同名时私有技能排在后面（全局优先）。
pub(crate) fn load_skills_for_conversation(
    app: &AppHandle,
    bundle: Option<&crate::llm::services::agent_bundles::AgentBundle>,
) -> Result<Vec<SkillEntry>, String> {
    let mut skills: Vec<SkillEntry> = load_enabled_skills_with_app(app)?
        .into_iter()
        .filter(|s| bundle.map(|b| b.is_skill_enabled(&s.name)).unwrap_or(true))
        .collect();

    if let Some(bundle) = bundle {
        skills.extend(load_agent_private_skills(app, &bundle.id)?);
    }
    Ok(skills)
}

// 返回给前端、系统提示词等模块使用的轻量 skill 摘要。
pub fn list_skill_summaries_with_app(app: &AppHandle) -> Result<Vec<SkillSummary>, String> {
    Ok(to_summaries(load_skills_with_app(app)?))
}

/// 某智能体的私有技能摘要（前端智能体配置页用）。
pub fn list_agent_private_skill_summaries(
    app: &AppHandle,
    bundle_id: &str,
) -> Result<Vec<SkillSummary>, String> {
    Ok(to_summaries(load_agent_private_skills(app, bundle_id)?))
}

// 仅返回已启用技能的摘要：系统提示词注入用。
pub fn list_enabled_skill_summaries_with_app(
    app: &AppHandle,
) -> Result<Vec<SkillSummary>, String> {
    Ok(to_summaries(load_enabled_skills_with_app(app)?))
}

fn to_summaries(skills: Vec<SkillEntry>) -> Vec<SkillSummary> {
    skills
        .into_iter()
        .map(|s| SkillSummary {
            name: s.name,
            description: s.description,
            path: s.path.display().to_string(),
        })
        .collect()
}

// 收集某个 skill 所在目录下的附属文件列表。
pub(crate) fn collect_skill_sibling_files(skill_md_path: &Path) -> Vec<(String, PathBuf)> {
    let Some(skill_dir) = skill_md_path.parent() else {
        return vec![];
    };

    let mut result = Vec::new();
    collect_dir_files_relative(skill_dir, skill_dir, &mut result);
    result
        .into_iter()
        .filter(|(rel, _)| {
            let lower = rel.to_ascii_lowercase();
            lower != "skill.md" && !lower.ends_with("license.txt")
        })
        .collect()
}

// 递归收集目录里的普通文件，并记录相对路径。
fn collect_dir_files_relative(root: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
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
