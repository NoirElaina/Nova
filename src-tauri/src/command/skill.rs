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

        // 安全校验：确保目录在 app_data_dir/skills 下，防止路径穿越。
        let skills_root = app
            .path()
            .app_data_dir()
            .map(|d| d.join("skills"))
            .map_err(|e| format!("无法解析 skills 根目录: {}", e))?;

        if !skill_dir.starts_with(&skills_root) {
            return Err("拒绝删除 skills 目录之外的路径".to_string());
        }

        if !skill_dir.exists() {
            return Err("技能目录不存在".to_string());
        }

        std::fs::remove_dir_all(&skill_dir).map_err(|e| format!("删除技能目录失败: {}", e))
    })();
    report_backend_result(&app, "command.skill.delete_skill", result, None)
}
