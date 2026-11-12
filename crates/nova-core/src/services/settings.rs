pub use nova_protocol::settings::*;

/// 计算指定模型的上下文窗口（用户设置覆盖 > 内置模型库 > 默认）。
/// 原 AppSettings 方法的领域自由函数形态（线类型不含内核依赖）。
pub fn context_window_for_model(settings: &AppSettings, model: &str) -> u32 {
    crate::agent::utils::model_context::resolve_context_window_tokens(
        model,
        &settings.model_context_windows,
    )
}

use std::collections::HashMap;
use std::path::PathBuf;
use crate::core::Core;
use tracing::warn;

pub fn get_settings_path(core: &Core) -> Result<PathBuf, String> {
    Ok(core.config().data_dir().join("settings.json"))
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

const MAX_SETTINGS_CACHE_ENTRIES: usize = 64;

static SETTINGS_CACHE: std::sync::Mutex<
    Option<HashMap<PathBuf, (crate::agent::utils::fingerprint::FileFingerprint, AppSettings)>>,
> = std::sync::Mutex::new(None);

pub fn invalidate_settings_cache() {
    if let Ok(mut guard) = SETTINGS_CACHE.lock() {
        *guard = None;
    }
}

pub fn load_settings(core: &Core) -> Result<AppSettings, String> {
    let path = get_settings_path(core)?;

    if !path.exists() {
        let mut settings = AppSettings::default();
        settings.normalize_for_runtime();
        return Ok(settings);
    }

    let fingerprint = crate::agent::utils::fingerprint::FileFingerprint::of(&path);
    if let Some(fp) = fingerprint {
        if let Ok(guard) = SETTINGS_CACHE.lock() {
            if let Some((cached_fp, cached_settings)) =
                guard.as_ref().and_then(|cache| cache.get(&path))
            {
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
    if crate::services::settings_secrets::has_plaintext_provider_api_keys(&settings) {
        let mut persisted = settings.clone();
        match crate::services::settings_secrets::encrypt_provider_api_keys(&mut persisted)
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

    crate::services::settings_secrets::decrypt_provider_api_keys(&mut settings);

    if let Some(fp) = crate::agent::utils::fingerprint::FileFingerprint::of(&path) {
        if let Ok(mut guard) = SETTINGS_CACHE.lock() {
            let cache = guard.get_or_insert_with(HashMap::new);
            if cache.len() >= MAX_SETTINGS_CACHE_ENTRIES && !cache.contains_key(&path) {
                cache.clear();
            }
            cache.insert(path.clone(), (fp, settings.clone()));
        }
    }
    crate::services::terminal_detector::set_configured_terminal(settings.terminal_shell.clone());
    Ok(settings)
}

pub fn save_settings_inner(core: &Core, settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path(core)?;
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(e.to_string());
        }
    }
    let mut normalized = settings;
    normalized.normalize_for_runtime();
    validate_provider_profiles(&normalized)?;
    crate::services::settings_secrets::encrypt_provider_api_keys(&mut normalized)?;
    let content = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    if let Ok(mut guard) = SETTINGS_CACHE.lock() {
        if let Some(cache) = guard.as_mut() {
            cache.remove(&path);
        }
    }
    crate::services::terminal_detector::set_configured_terminal(normalized.terminal_shell.clone());
    core.hooks().set_file_logging_enabled(normalized.enable_app_log);
    Ok(())
}
