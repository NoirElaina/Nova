use crate::llm::services::skills::{
    collect_skill_sibling_files, load_skills_with_app, normalize_skill_name, truncate_chars,
    SkillEntry,
};
use crate::llm::tools::{app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

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
pub(super) fn registration() -> ToolRegistration {
    app_tool(tool, execute_with_app_boxed, true, None)
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

// 按名字挑出一个 skill，并把正文和附属文件说明打包成返回文本。
// `skill_name` 是模型请求的技能名，`args` 是附加给 skill 的可选参数提示。
fn run_skill(
    skills: &[SkillEntry],
    skill_name: &str,
    args: Option<&str>,
) -> Result<ToolOutcome, ToolFailure> {
    // skill_name_norm: 归一化后的名字，用于大小写不敏感查找。
    let skill_name_norm = normalize_skill_name(skill_name);

    let picked = skills
        .iter()
        .find(|s| normalize_skill_name(&s.name) == skill_name_norm)
        .or_else(|| {
            skills
                .iter()
                .find(|s| normalize_skill_name(&s.name).contains(&skill_name_norm))
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

    let skills = match load_skills_with_app(app) {
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
