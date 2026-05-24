use std::collections::HashMap;
use tauri::AppHandle;

#[derive(Debug, Clone, Default)]
pub(crate) struct HookConfig {
    values: HashMap<String, String>,
}

impl HookConfig {
    pub(crate) fn from_app(app: &AppHandle) -> Result<Self, String> {
        Ok(Self {
            values: crate::command::settings::get_settings(app.clone())?.hook_env,
        })
    }

    pub(crate) fn value(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    }

    pub(crate) fn truthy(&self, key: &str) -> bool {
        self.value(key)
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    }

    pub(crate) fn csv_lower_list(&self, key: &str) -> Vec<String> {
        self.value(key)
            .map(|v| {
                v.split(',')
                    .map(|part| part.trim().to_ascii_lowercase())
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
}
