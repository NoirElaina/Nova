use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tracing::warn;

fn default_custom_models() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

fn default_provider_profiles() -> HashMap<String, ProviderProfile> {
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
    "on_request".to_string()
}

fn default_progressive_tool_disclosure() -> bool {
    true
}

pub fn normalize_provider_key(provider: &str) -> String {
    let key = provider.trim().to_ascii_lowercase();
    if key.is_empty() {
        "anthropic".to_string()
    } else {
        key
    }
}

pub fn normalize_provider_api_format(api_format: &str) -> String {
    match api_format.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "anthropic".to_string(),
        "openai_responses" | "responses" => "openai_responses".to_string(),
        _ => "openai".to_string(),
    }
}

pub fn infer_provider_api_format(provider_key: &str) -> String {
    match provider_key.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "anthropic".to_string(),
        _ => "openai".to_string(),
    }
}

pub fn normalize_ui_language(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "english" => "en-US".to_string(),
        _ => "zh-CN".to_string(),
    }
}

pub fn normalize_ui_theme(raw: &str) -> String {
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
    pub display_name: String,
    #[serde(default)]
    #[serde(alias = "protocol")]
    pub api_format: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub anthropic_thinking_enabled: bool,
    #[serde(default)]
    pub anthropic_thinking_budget_tokens: Option<u32>,
    #[serde(default = "default_stop_sequences")]
    pub stop_sequences: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct RagSettings {
    #[serde(default)]
    pub embedding_model: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub provider: String,
    #[serde(default = "default_custom_models")]
    pub custom_models: HashMap<String, Vec<String>>,
    #[serde(default = "default_provider_profiles")]
    pub provider_profiles: HashMap<String, ProviderProfile>,
    #[serde(default)]
    pub provider_order: Vec<String>,
    #[serde(default)]
    pub model_context_windows: HashMap<String, u32>,
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    #[serde(default = "default_rag_settings")]
    pub rag: RagSettings,
    #[serde(default = "default_ui_language")]
    pub ui_language: String,
    #[serde(default = "default_ui_theme")]
    pub ui_theme: String,
    #[serde(default = "default_enable_app_log")]
    pub enable_app_log: bool,
    #[serde(default = "default_approval_policy")]
    pub approval_policy: String,
    #[serde(default = "default_progressive_tool_disclosure")]
    pub progressive_tool_disclosure: bool,
    #[serde(default)]
    pub terminal_shell: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
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
            approval_policy: default_approval_policy(),
            progressive_tool_disclosure: default_progressive_tool_disclosure(),
            terminal_shell: None,
        }
    }
}

impl AppSettings {
    pub fn context_window_for_model(&self, model: &str) -> u32 {
        crate::agent::utils::model_context::resolve_context_window_tokens(
            model,
            &self.model_context_windows,
        )
    }

    pub fn active_provider_key(&self) -> String {
        normalize_provider_key(&self.provider)
    }

    pub fn active_provider_profile(&self) -> ProviderProfile {
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
        let key = self.active_provider_key();
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

        self.rag.embedding_model = self.rag.embedding_model.trim().to_string();

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

        self.ui_language = normalize_ui_language(&self.ui_language);
        self.ui_theme = normalize_ui_theme(&self.ui_theme);

        if crate::agent::permissions::ApprovalPolicy::parse(&self.approval_policy).is_none() {
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
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("settings.json"))
        .map_err(|e| format!("Failed to resolve app_data_dir for settings: {}", e))
}

pub fn validate_rag_settings(settings: &AppSettings) -> Result<(), String> {
    let rag = &settings.rag;
    if rag.embedding_model.chars().count() > 256 {
        return Err("Invalid rag.embeddingModel: too long".to_string());
    }
    Ok(())
}

pub fn validate_provider_profiles(settings: &AppSettings) -> Result<(), String> {
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

static SETTINGS_CACHE: std::sync::Mutex<
    Option<(crate::agent::utils::fingerprint::FileFingerprint, AppSettings)>,
> = std::sync::Mutex::new(None);

pub fn invalidate_settings_cache() {
    if let Ok(mut guard) = SETTINGS_CACHE.lock() {
        *guard = None;
    }
}

pub fn load_settings(app: &AppHandle) -> Result<AppSettings, String> {
    let path = get_settings_path(app)?;

    if !path.exists() {
        let mut settings = AppSettings::default();
        settings.normalize_for_runtime();
        return Ok(settings);
    }

    let fingerprint = crate::agent::utils::fingerprint::FileFingerprint::of(&path);
    if let Some(fp) = fingerprint {
        if let Ok(guard) = SETTINGS_CACHE.lock() {
            if let Some((cached_fp, cached_settings)) = guard.as_ref() {
                if *cached_fp == fp {
                    return Ok(cached_settings.clone());
                }
            }
        }
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
                operation = "services.settings.load_settings",
                path = %path.display(),
                error = %error,
                "failed to migrate plaintext API keys"
            ),
        }
    }

    crate::command::settings_secrets::decrypt_provider_api_keys(&mut settings);

    if let Some(fp) = crate::agent::utils::fingerprint::FileFingerprint::of(&path) {
        if let Ok(mut guard) = SETTINGS_CACHE.lock() {
            *guard = Some((fp, settings.clone()));
        }
    }
    crate::services::terminal_detector::set_configured_terminal(settings.terminal_shell.clone());
    Ok(settings)
}

pub fn save_settings_inner(app: &AppHandle, settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path(app)?;
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(e.to_string());
        }
    }
    let mut normalized = settings;
    normalized.normalize_for_runtime();
    validate_rag_settings(&normalized)?;
    validate_provider_profiles(&normalized)?;
    crate::command::settings_secrets::encrypt_provider_api_keys(&mut normalized)?;
    let content = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    invalidate_settings_cache();
    crate::services::terminal_detector::set_configured_terminal(normalized.terminal_shell.clone());
    crate::logging::set_file_logging_enabled(normalized.enable_app_log);
    Ok(())
}
