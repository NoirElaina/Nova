//! 脱敏诊断包导出服务：汇总设置、沙箱驱动状态、Wire Trace 与最新应用日志并清理凭据。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nova_protocol::redact::{redact_json_value, redact_secrets};
use serde_json::{json, Value};

use crate::agent::sandbox::active_status;
use crate::core::Core;
use crate::services::settings::load_settings;

const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_LOG_LINES: usize = 200;

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_redacted_log_tail(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    let slice = if bytes.len() > MAX_LOG_BYTES {
        &bytes[bytes.len() - MAX_LOG_BYTES..]
    } else {
        &bytes[..]
    };
    let text = String::from_utf8_lossy(slice);
    let lines: Vec<String> = text
        .lines()
        .rev()
        .take(MAX_LOG_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(redact_secrets)
        .collect();
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app.log")
        .to_string();
    Some(json!({
        "file": name,
        "lines": lines,
    }))
}

fn collect_recent_logs(data_dir: &Path) -> Value {
    let log_dir = data_dir.join("logs");
    let Ok(entries) = fs::read_dir(&log_dir) else {
        return json!({ "files": [] });
    };

    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            Some((modified, path))
        })
        .collect();

    files.sort_by(|a, b| b.0.cmp(&a.0));
    let tails: Vec<Value> = files
        .into_iter()
        .take(3)
        .filter_map(|(_, path)| read_redacted_log_tail(&path))
        .collect();

    json!({ "files": tails })
}

/// 导出脱敏后的本地诊断包 JSON 文件，并返回生成的文件路径。
pub fn export_diagnostic_bundle(core: &Core, target_path: Option<String>) -> Result<String, String> {
    let data_dir = core.config().data_dir();
    let out_path = match target_path.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(custom) => PathBuf::from(custom),
        None => data_dir
            .join("diagnostics")
            .join(format!("nova-diagnostics-{}.json", now_unix_secs())),
    };

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create diagnostic directory: {e}"))?;
    }

    let settings_json = match load_settings(core) {
        Ok(settings) => {
            let raw = serde_json::to_value(&settings).unwrap_or(Value::Null);
            redact_json_value(&raw)
        }
        Err(err) => json!({ "loadError": redact_secrets(&err) }),
    };

    let wire_traces_raw = serde_json::to_value(core.recent_wire_traces()).unwrap_or(Value::Array(vec![]));
    let wire_traces = redact_json_value(&wire_traces_raw);

    let bundle = json!({
        "schemaVersion": 1,
        "generatedAtUnixSecs": now_unix_secs(),
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "sandbox": active_status(core),
        "settings": settings_json,
        "wireTraces": wire_traces,
        "logs": collect_recent_logs(&data_dir),
    });

    let encoded = serde_json::to_string_pretty(&bundle)
        .map_err(|e| format!("Failed to serialize diagnostic bundle: {e}"))?;
    fs::write(&out_path, encoded)
        .map_err(|e| format!("Failed to write diagnostic bundle: {e}"))?;

    Ok(out_path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CoreConfig;
    use nova_protocol::settings::{AppSettings, ProviderProfile};

    #[test]
    fn export_diagnostic_bundle_redacts_secrets_from_settings_and_logs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = CoreConfig::new(dir.path().to_path_buf());
        let core = Core::new(cfg, 16);

        let mut settings = AppSettings::default();
        settings.provider_profiles.insert(
            "p1".into(),
            ProviderProfile {
                display_name: "SecretProvider".into(),
                api_key: "sk-proj-supersecretkeyvalue12345".into(),
                base_url: "https://api.example.com".into(),
                model: "gpt-4o".into(),
                ..Default::default()
            },
        );
        crate::services::settings::invalidate_settings_cache();
        let settings_path = crate::services::settings::get_settings_path(&core).expect("settings path");
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).expect("serialize settings"),
        )
        .expect("write settings.json");

        let log_dir = dir.path().join("logs");
        fs::create_dir_all(&log_dir).expect("log dir");
        fs::write(
            log_dir.join("app.log.2026-11-12"),
            "INFO start\nERROR Authorization: Bearer secret-bearer-token-99999\n",
        )
        .expect("write log");

        let out_file = dir.path().join("diag-out.json");
        let exported = export_diagnostic_bundle(&core, Some(out_file.display().to_string()))
            .expect("export bundle");
        assert_eq!(exported, out_file.display().to_string());

        let content = fs::read_to_string(&out_file).expect("read exported bundle");
        assert!(!content.contains("sk-proj-supersecretkeyvalue12345"));
        assert!(!content.contains("secret-bearer-token-99999"));
        assert!(content.contains("[REDACTED]"));
        assert!(content.contains("\"sandbox\""));
    }
}
