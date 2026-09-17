use crate::llm::services::skills::{list_skill_summaries_with_app, SkillSummary};
use crate::llm::utils::error_event::report_backend_result;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

#[tauri::command]
pub fn list_skills(app: AppHandle) -> Result<Vec<SkillSummary>, String> {
    // 返回技能摘要列表。
    report_backend_result(
        &app,
        "command.skill.list_skills",
        list_skill_summaries_with_app(&app),
        None,
    )
}

#[tauri::command]
pub fn delete_skill(app: AppHandle, path: String) -> Result<(), String> {
    let result = (|| {
        // `path` 是 SKILL.md 的绝对路径，技能目录是其父目录。
        let skill_md = PathBuf::from(&path);
        let skill_dir = skill_md
            .parent()
            .ok_or_else(|| "无法解析技能目录".to_string())?
            .to_path_buf();

        // 安全校验：目录必须位于全局 skills 根目录或某个智能体的私有技能目录下，
        // 防止路径穿越。
        let app_data = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("无法解析应用数据目录: {}", e))?;
        let skills_root = app_data.join("skills");

        let allowed = skill_dir.starts_with(&skills_root)
            || crate::llm::services::agent_bundles::list_bundles(&app)
                .map(|bundles| {
                    bundles.iter().any(|b| {
                        agent_bundle_skills_dir(&app, &b.id)
                            .map(|dir| skill_dir.starts_with(&dir))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
        if !allowed {
            return Err("拒绝删除 skills 目录之外的路径".to_string());
        }

        if !skill_dir.exists() {
            return Err("技能目录不存在".to_string());
        }

        // 删除前解析技能名（与列表展示口径一致：frontmatter name，缺省用目录名），
        // 用于删除后清理 disabledSkills 中的残留条目。
        let skill_name = std::fs::read_to_string(&skill_md)
            .ok()
            .map(|raw| crate::llm::services::skills::pick_skill_name(&skill_md, &raw))
            .or_else(|| {
                skill_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            });

        std::fs::remove_dir_all(&skill_dir).map_err(|e| format!("删除技能目录失败: {}", e))?;

        // 从停用名单移除该技能名（对标插件卸载的清理逻辑：残留无害但顺手清干净）。
        if let Some(name) = skill_name {
            if let Ok(mut settings) = crate::command::settings::load_settings(&app) {
                let target = crate::llm::services::skills::normalize_skill_name(&name);
                let before = settings.disabled_skills.len();
                settings
                    .disabled_skills
                    .retain(|existing| {
                        crate::llm::services::skills::normalize_skill_name(existing) != target
                    });
                if settings.disabled_skills.len() != before {
                    let _ = crate::command::settings::save_settings_inner(&app, settings);
                }
            }
        }

        Ok(())
    })();
    report_backend_result(&app, "command.skill.delete_skill", result, None)
}

fn agent_bundle_skills_dir(
    app: &AppHandle,
    bundle_id: &str,
) -> Result<PathBuf, String> {
    crate::llm::services::agent_bundles::agent_skills_dir(app, bundle_id)
}
