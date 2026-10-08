use tauri::AppHandle;
use crate::agent::utils::error_event::report_backend_result;

// Re-export all domain types and functions from the core settings service
pub use crate::services::settings::*;

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    let result = load_settings(&app);
    report_backend_result(&app, "command.settings.get_settings", result, None)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    let result = save_settings_inner(&app, settings);
    report_backend_result(&app, "command.settings.save_settings", result, None)
}

/// 返回指定模型名对应的上下文窗口大小（token 数）。
/// 优先级：用户设置覆盖 > 内置 models.json > 默认 200K。
/// 前端在无活跃对话时用此命令初始化 ContextUsageIndicator 的分母。
#[tauri::command]
pub fn get_model_window_tokens(app: AppHandle, model: String) -> u32 {
    match load_settings(&app) {
        Ok(settings) => settings.context_window_for_model(&model),
        Err(_) => crate::agent::utils::model_context::get_context_window_tokens(&model),
    }
}

/// 文本 token 数：全项目标准计数器（o200k_base BPE 分词器）。
/// 不区分协议/模型——分词是模型能力而非协议差异，统一基准即可。
#[tauri::command]
pub fn estimate_text_tokens(text: String) -> u32 {
    crate::provider::token_counter::count_text(&text).clamp(0, u32::MAX as i64) as u32
}

/// 查询本机已探测到的可用终端与 Shell 列表
#[tauri::command]
pub fn get_available_terminals() -> Vec<crate::services::terminal_detector::TerminalInfo> {
    crate::services::terminal_detector::list_available_terminals()
}

/// 立即从 OpenRouter 刷新模型元数据与定价数据库
#[tauri::command]
pub async fn refresh_model_catalog(app: AppHandle) -> Result<crate::services::model_catalog::ModelCatalogStats, String> {
    crate::services::model_catalog::force_refresh(&app).await
}

/// 获取当前模型元数据数据库的状态统计
#[tauri::command]
pub fn get_model_catalog_stats(app: AppHandle) -> crate::services::model_catalog::ModelCatalogStats {
    crate::services::model_catalog::get_catalog_stats(&app)
}

/// 查询模型列表（支持关键字搜索与模态过滤）
#[tauri::command]
pub fn get_model_catalog(
    search: Option<String>,
    modality: Option<String>,
) -> Vec<crate::services::model_catalog::ModelCatalogSummary> {
    crate::services::model_catalog::list_models(search.as_deref(), modality.as_deref())
}

