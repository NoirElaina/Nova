use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tracing::warn;

use crate::llm::utils::error_event::report_backend_result;

fn default_custom_models() -> HashMap<String, Vec<String>> {
    // custom_models 默认空映射。
    HashMap::new()
}

fn default_provider_profiles() -> HashMap<String, ProviderProfile> {
    // provider_profiles 默认空映射。
    HashMap::new()
}

fn default_rag_settings() -> RagSettings {
    RagSettings::default()
}

fn default_ui_language() -> String {
    "zh-CN".to_string()
}

fn default_ui_theme() -> String {
    "system".to_string()
}

fn default_enable_app_log() -> bool {
    false
}

fn default_stop_sequences() -> Vec<String> {
    Vec::new()
}

fn default_approval_policy() -> String {
    // 默认策略：仅风险操作（Risky）才弹审批。
    "on_request".to_string()
}

fn default_progressive_tool_disclosure() -> bool {
    // 默认开启渐进式工具披露：低频工具按需加载，减少提示词体积。
    true
}

fn normalize_provider_key(provider: &str) -> String {
    // provider 名去空白并转小写。
    let key = provider.trim().to_ascii_lowercase();
    // 空 provider 回退 anthropic。
    if key.is_empty() {
        "anthropic".to_string()
    } else {
        // 返回规范化 provider key。
        key
    }
}

fn normalize_provider_api_format(api_format: &str) -> String {
    match api_format.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "anthropic".to_string(),
        "openai_responses" | "responses" => "openai_responses".to_string(),
        _ => "openai".to_string(),
    }
}

fn infer_provider_api_format(provider_key: &str) -> String {
    match provider_key.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "anthropic".to_string(),
        _ => "openai".to_string(),
    }
}

fn normalize_ui_language(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "english" => "en-US".to_string(),
        _ => "zh-CN".to_string(),
    }
}

fn normalize_ui_theme(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "light" => "light".to_string(),
        "dark" => "dark".to_string(),
        _ => "system".to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    #[serde(default)]
    // UI 展示名。
    pub display_name: String,
    #[serde(default)]
    #[serde(alias = "protocol")]
    // 接口格式：openai / anthropic / openai_responses。
    pub api_format: String,
    #[serde(default)]
    // provider API key。
    pub api_key: String,
    #[serde(default)]
    // provider base_url。
    pub base_url: String,
    #[serde(default)]
    // provider model。
    pub model: String,
    #[serde(default)]
    // Anthropic extended thinking 是否启用。
    pub anthropic_thinking_enabled: bool,
    #[serde(default)]
    // Anthropic extended thinking token 预算。
    pub anthropic_thinking_budget_tokens: Option<u32>,
    #[serde(default = "default_stop_sequences")]
    // Anthropic stop_sequences / 其他协议可复用的停止序列。
    pub stop_sequences: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagSettings {
    #[serde(default)]
    // embedding 模型名称。
    pub embedding_model: String,
}

impl Default for RagSettings {
    fn default() -> Self {
        Self {
            embedding_model: String::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    // 当前 provider 标识。
    pub provider: String,
    #[serde(default = "default_custom_models")]
    // 各 provider 的自定义模型列表。
    pub custom_models: HashMap<String, Vec<String>>,
    #[serde(default = "default_provider_profiles")]
    // 各 provider 的独立配置。
    pub provider_profiles: HashMap<String, ProviderProfile>,
    #[serde(default)]
    // 模型配置页展示顺序（provider id 列表）。
    pub provider_order: Vec<String>,
    #[serde(default)]
    // 按模型名覆盖上下文窗口（token）。优先于内置 models.json。
    pub model_context_windows: HashMap<String, u32>,
    #[serde(default)]
    // 被禁用的技能列表。
    pub disabled_skills: Vec<String>,
    #[serde(default = "default_rag_settings")]
    // RAG 相关配置。
    pub rag: RagSettings,
    #[serde(default = "default_ui_language")]
    // UI 语言（zh-CN/en-US）。
    pub ui_language: String,
    #[serde(default = "default_ui_theme")]
    // UI 主题（system/light/dark）。
    pub ui_theme: String,
    #[serde(default = "default_enable_app_log")]
    // 是否记录统一软件日志到文件。
    pub enable_app_log: bool,
    #[serde(default)]
    // 被停用的插件 id 列表。
    pub disabled_plugins: Vec<String>,
    #[serde(default = "default_approval_policy")]
    // 审批策略：always_ask / on_request / never。
    pub approval_policy: String,
    #[serde(default = "default_progressive_tool_disclosure")]
    // 渐进式工具披露：低频工具不进提示词，由模型通过 LoadTool 按需加载。
    pub progressive_tool_disclosure: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        // 应用设置默认值。
        Self {
            provider: "anthropic".to_string(),
            custom_models: HashMap::new(),
            provider_profiles: HashMap::new(),
            provider_order: Vec::new(),
            model_context_windows: HashMap::new(),
            disabled_skills: Vec::new(),
            rag: RagSettings::default(),
            ui_language: default_ui_language(),
            ui_theme: default_ui_theme(),
            enable_app_log: default_enable_app_log(),
            disabled_plugins: Vec::new(),
            approval_policy: default_approval_policy(),
            progressive_tool_disclosure: default_progressive_tool_disclosure(),
        }
    }
}

impl AppSettings {
    /// 当前模型最终上下文窗口：用户覆盖 > JSON 库 > 默认。
    pub fn context_window_for_model(&self, model: &str) -> u32 {
        crate::llm::utils::model_context::resolve_context_window_tokens(
            model,
            &self.model_context_windows,
        )
    }

    pub fn active_provider_key(&self) -> String {
        // 返回规范化后的当前 provider key。
        normalize_provider_key(&self.provider)
    }

    pub fn active_provider_profile(&self) -> ProviderProfile {
        // 计算当前 provider key。
        let key = self.active_provider_key();
        self.provider_profiles
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    pub fn active_provider_api_format(&self) -> String {
        let key = self.active_provider_key();
        let profile = self.provider_profiles.get(&key);
        let raw_api_format = profile
            .map(|profile| profile.api_format.trim())
            .filter(|api_format| !api_format.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| infer_provider_api_format(&key));
        normalize_provider_api_format(&raw_api_format)
    }

    pub fn normalize_for_runtime(&mut self) {
        // 规范化 provider key。
        let key = self.active_provider_key();
        // 将 provider 字段回写为规范化值。
        self.provider = key.clone();
        self.provider_profiles.entry(key.clone()).or_default();

        for (profile_key, profile) in self.provider_profiles.iter_mut() {
            if profile.api_format.trim().is_empty() {
                profile.api_format = infer_provider_api_format(profile_key);
            } else {
                profile.api_format = normalize_provider_api_format(&profile.api_format);
            }
            profile.display_name = profile.display_name.trim().to_string();
            profile.stop_sequences = profile
                .stop_sequences
                .iter()
                .map(|sequence| sequence.trim().to_string())
                .filter(|sequence| !sequence.is_empty())
                .collect();
        }

        // 规范化 RAG 配置。
        self.rag.embedding_model = self.rag.embedding_model.trim().to_string();

        // 规范化模型上下文覆盖：去空白键、丢弃 0、钳制到合理范围。
        const MIN_CTX: u32 = 1_024;
        const MAX_CTX: u32 = 16_000_000;
        let mut normalized_windows = HashMap::new();
        for (model, tokens) in self.model_context_windows.drain() {
            let name = model.trim().to_string();
            if name.is_empty() || tokens == 0 {
                continue;
            }
            normalized_windows.insert(name, tokens.clamp(MIN_CTX, MAX_CTX));
        }
        self.model_context_windows = normalized_windows;

        self.sync_provider_order();

        // 规范化 UI 偏好配置。
        self.ui_language = normalize_ui_language(&self.ui_language);
        self.ui_theme = normalize_ui_theme(&self.ui_theme);

        // 规范化审批策略：非法值回落默认。
        if crate::llm::utils::permissions::ApprovalPolicy::parse(&self.approval_policy).is_none() {
            self.approval_policy = default_approval_policy();
        }
    }

    fn sync_provider_order(&mut self) {
        self.provider_order
            .retain(|id| self.provider_profiles.contains_key(id));

        let mut missing: Vec<String> = self
            .provider_profiles
            .keys()
            .filter(|id| !self.provider_order.iter().any(|existing| existing == *id))
            .cloned()
            .collect();
        missing.sort();
        self.provider_order.extend(missing);
    }
}

pub fn get_settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    // 设置文件路径严格使用 app_data_dir，不再提供回退路径。
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("settings.json"))
        .map_err(|e| format!("Failed to resolve app_data_dir for settings: {}", e))
}

fn validate_rag_settings(settings: &AppSettings) -> Result<(), String> {
    let rag = &settings.rag;

    if rag.embedding_model.chars().count() > 256 {
        return Err("Invalid rag.embeddingModel: too long".to_string());
    }

    Ok(())
}

fn validate_provider_profiles(settings: &AppSettings) -> Result<(), String> {
    for (profile_key, profile) in &settings.provider_profiles {
        if let Some(budget) = profile.anthropic_thinking_budget_tokens {
            if budget < 1024 {
                return Err(format!(
                    "Invalid providerProfiles[{}].anthropicThinkingBudgetTokens: must be at least 1024",
                    profile_key
                ));
            }
        }

        for sequence in &profile.stop_sequences {
            if sequence.contains('\u{0000}') {
                return Err(format!(
                    "Invalid providerProfiles[{}].stopSequences: contains NUL character",
                    profile_key
                ));
            }
            if sequence.chars().count() > 256 {
                return Err(format!(
                    "Invalid providerProfiles[{}].stopSequences: sequence is too long",
                    profile_key
                ));
            }
        }
    }

    Ok(())
}

/// 内部加载设置（不走 command 错误上报），供 query/compact/window tokens 等路径复用。
pub fn load_settings(app: &AppHandle) -> Result<AppSettings, String> {
    let path = get_settings_path(app)?;

    // 首次启动还没有 settings.json 时，返回运行时默认配置。
    if !path.exists() {
        let mut settings = AppSettings::default();
        settings.normalize_for_runtime();
        return Ok(settings);
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取设置文件失败 {}: {}", path.display(), error))?;
    let mut settings = serde_json::from_str::<AppSettings>(&content)
        .map_err(|error| format!("解析设置文件失败 {}: {}", path.display(), error))?;

    settings.normalize_for_runtime();
    if crate::command::settings_secrets::has_plaintext_provider_api_keys(&settings) {
        let mut persisted = settings.clone();
        match crate::command::settings_secrets::encrypt_provider_api_keys(&mut persisted)
            .and_then(|_| {
                serde_json::to_string_pretty(&persisted).map_err(|error| error.to_string())
            })
            .and_then(|content| std::fs::write(&path, content).map_err(|error| error.to_string()))
        {
            Ok(()) => {}
            Err(error) => warn!(
                operation = "command.settings.load_settings",
                path = %path.display(),
                error = %error,
                "failed to migrate plaintext API keys"
            ),
        }
    }
    crate::command::settings_secrets::decrypt_provider_api_keys(&mut settings);
    Ok(settings)
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    let result = load_settings(&app);
    report_backend_result(&app, "command.settings.get_settings", result, None)
}

/// 内部保存设置：Rust 侧调用（如 agent bundle 激活切换），不走 command 错误上报。
pub fn save_settings_inner(app: &AppHandle, settings: AppSettings) -> Result<(), String> {
    // 获取 settings.json 路径。
    let path = get_settings_path(app)?;
    // 确保父目录存在。
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(e.to_string());
        }
    }
    // 对传入设置做运行时规范化。
    let mut normalized = settings;
    normalized.normalize_for_runtime();
    validate_rag_settings(&normalized)?;
    validate_provider_profiles(&normalized)?;
    crate::command::settings_secrets::encrypt_provider_api_keys(&mut normalized)?;
    // 序列化为美化 JSON。
    let content = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
    // 写入文件。
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    crate::logging::set_file_logging_enabled(normalized.enable_app_log);
    Ok(())
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
        Err(_) => crate::llm::utils::model_context::get_context_window_tokens(&model),
    }
}

/// 文本 token 数：全项目标准计数器（o200k_base BPE 分词器）。
/// 不区分协议/模型——分词是模型能力而非协议差异，统一基准即可。
#[tauri::command]
pub fn estimate_text_tokens(text: String) -> u32 {
    crate::llm::utils::token_counter::count_text(&text).clamp(0, u32::MAX as i64) as u32
}
