use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_custom_models() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

fn default_provider_profiles() -> HashMap<String, ProviderProfile> {
    HashMap::new()
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

        if crate::agent::ApprovalPolicy::parse(&self.approval_policy).is_none() {
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
