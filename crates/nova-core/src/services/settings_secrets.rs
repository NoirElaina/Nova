use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use crate::core::Core;
use tracing::{info, warn};

use crate::services::settings::AppSettings;

/// 现代加密密钥前缀
const SECRET_PREFIX: &str = "nova:v2:";
/// 旧版 DPAPI 加密密钥前缀（兼容提示）
const LEGACY_DPAPI_PREFIX: &str = "nova:dpapi:v1:";
/// 旧版明文主密钥文件名（迁移后删除；仅无凭据服务的无头 Linux 环境回退保留）
const MASTER_KEY_FILENAME: &str = "master_key";
/// OS 凭据库服务标识
const KEYRING_SERVICE: &str = "com.tauri-app.nova";
/// AES-256-GCM nonce 长度（12 字节 / 96 位）
const NONCE_LEN: usize = 12;
/// AES-256 密钥长度（32 字节 / 256 位）
const KEY_LEN: usize = 32;

/// 进程内缓存的主加密密钥
static MASTER_KEY: OnceLock<[u8; KEY_LEN]> = OnceLock::new();

#[derive(Debug, PartialEq, Eq)]
enum KeyringLookup {
    Found(String),
    Missing,
    Unavailable(String),
}

#[derive(Debug, PartialEq, Eq)]
enum KeyringWriteOutcome {
    Stored,
    Unavailable(String),
}

trait KeyringBackend {
    fn get(&self, service: &str, account: &str) -> Result<KeyringLookup, String>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<KeyringWriteOutcome, String>;
}

struct OsKeyringBackend;

impl KeyringBackend for OsKeyringBackend {
    fn get(&self, service: &str, account: &str) -> Result<KeyringLookup, String> {
        let entry = match keyring::Entry::new(service, account) {
            Ok(entry) => entry,
            Err(keyring::Error::NoStorageAccess(err)) | Err(keyring::Error::PlatformFailure(err)) => {
                return Ok(KeyringLookup::Unavailable(err.to_string()));
            }
            Err(err) => return Err(format!("Failed to open OS credential entry: {err}")),
        };
        match entry.get_password() {
            Ok(secret) => Ok(KeyringLookup::Found(secret)),
            Err(keyring::Error::NoEntry) => Ok(KeyringLookup::Missing),
            Err(keyring::Error::NoStorageAccess(err)) | Err(keyring::Error::PlatformFailure(err)) => {
                Ok(KeyringLookup::Unavailable(err.to_string()))
            }
            Err(err) => Err(format!("Failed to read master key from OS credential store: {err}")),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<KeyringWriteOutcome, String> {
        let entry = match keyring::Entry::new(service, account) {
            Ok(entry) => entry,
            Err(keyring::Error::NoStorageAccess(err)) | Err(keyring::Error::PlatformFailure(err)) => {
                return Ok(KeyringWriteOutcome::Unavailable(err.to_string()));
            }
            Err(err) => return Err(format!("Failed to open OS credential entry: {err}")),
        };
        match entry.set_password(secret) {
            Ok(()) => Ok(KeyringWriteOutcome::Stored),
            Err(keyring::Error::NoStorageAccess(err)) | Err(keyring::Error::PlatformFailure(err)) => {
                Ok(KeyringWriteOutcome::Unavailable(err.to_string()))
            }
            Err(err) => Err(format!("Failed to store master key in OS credential store: {err}")),
        }
    }
}

/// 初始化主加密密钥，应用启动时必须优先调用
pub fn init_master_key(core: &Core) -> Result<(), String> {
    let data_dir = core.config().data_dir();
    let allow_file_fallback = cfg!(target_os = "linux")
        || std::env::var("NOVA_ALLOW_PLAINTEXT_MASTER_KEY")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let key = load_or_create_master_key_with(&data_dir, &OsKeyringBackend, allow_file_fallback)?;
    MASTER_KEY
        .set(key)
        .map_err(|_| "Master key already initialized".to_string())
}

/// 获取已初始化的主密钥；未初始化则报错
fn get_master_key() -> Result<[u8; KEY_LEN], String> {
    MASTER_KEY
        .get()
        .copied()
        .ok_or_else(|| "Master key not initialized; call init_master_key first".to_string())
}

#[cfg(test)]
pub(crate) fn init_ephemeral_test_master_key() {
    let _ = MASTER_KEY.set([0x42_u8; KEY_LEN]);
}

fn master_key_account(data_dir: &Path) -> String {
    let normalized = std::fs::canonicalize(data_dir)
        .unwrap_or_else(|_| data_dir.to_path_buf())
        .to_string_lossy()
        .into_owned();
    #[cfg(target_os = "windows")]
    let normalized = normalized.to_ascii_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let short_hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("master_key:{short_hex}")
}

fn decode_master_key_b64(encoded: &str, source: &str) -> Result<[u8; KEY_LEN], String> {
    let key_bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("Corrupt {source}: {e}"))?;
    key_bytes
        .try_into()
        .map_err(|_| format!("{source} has invalid key length"))
}

fn read_legacy_master_key(path: &Path) -> Result<[u8; KEY_LEN], String> {
    let key_b64 = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read master key file: {e}"))?;
    decode_master_key_b64(&key_b64, "master key file")
}

fn ensure_restrictive_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            warn!(path = %path.display(), error = %error, "failed to tighten master_key file permissions to 0600");
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn write_fallback_master_key_file(path: &Path, key_b64: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("Failed to open fallback master key file: {e}"))?;
        file.write_all(key_b64.as_bytes())
            .map_err(|e| format!("Failed to write fallback master key file: {e}"))?;
        ensure_restrictive_file_permissions(path);
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, key_b64)
            .map_err(|e| format!("Failed to write fallback master key file: {e}"))
    }
}

fn generate_random_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

fn verify_keyring_persisted(
    backend: &dyn KeyringBackend,
    account: &str,
    expected_key: &[u8; KEY_LEN],
) -> Result<(), String> {
    match backend.get(KEYRING_SERVICE, account)? {
        KeyringLookup::Found(stored_b64) => {
            let stored = decode_master_key_b64(&stored_b64, "OS credential store master key")?;
            if &stored == expected_key {
                Ok(())
            } else {
                Err("OS credential store readback mismatch after migration".to_string())
            }
        }
        KeyringLookup::Missing => {
            Err("OS credential store did not retain migrated master key".to_string())
        }
        KeyringLookup::Unavailable(reason) => {
            Err(format!("OS credential store became unavailable after write: {reason}"))
        }
    }
}

/// 优先从系统凭据库加载/创建主密钥；检测到旧明文 `master_key` 文件时一次性迁移并安全删除原文件。
/// 仅在显式允许回退的无凭据服务环境（如无头 Linux 容器/CI）保留 `0600` 文件回退。
fn load_or_create_master_key_with(
    data_dir: &Path,
    backend: &dyn KeyringBackend,
    allow_file_fallback: bool,
) -> Result<[u8; KEY_LEN], String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("Failed to create core data directory: {e}"))?;

    let account = master_key_account(data_dir);
    let legacy_path: PathBuf = data_dir.join(MASTER_KEY_FILENAME);

    match backend.get(KEYRING_SERVICE, &account)? {
        KeyringLookup::Found(secret_b64) => {
            let keyring_key =
                decode_master_key_b64(&secret_b64, "OS credential store master key")?;
            if legacy_path.exists() {
                match read_legacy_master_key(&legacy_path) {
                    Ok(file_key) if file_key == keyring_key => {
                        if let Err(error) = std::fs::remove_file(&legacy_path) {
                            warn!(
                                path = %legacy_path.display(),
                                error = %error,
                                "failed to remove redundant legacy master_key file"
                            );
                        }
                    }
                    Ok(_) => {
                        warn!(
                            path = %legacy_path.display(),
                            "OS credential store already holds master_key that differs from legacy file; keeping OS credential store key and leaving legacy file untouched"
                        );
                    }
                    Err(error) => {
                        warn!(
                            path = %legacy_path.display(),
                            error = %error,
                            "OS credential store holds master_key while legacy file could not be read; leaving legacy file untouched"
                        );
                    }
                }
            }
            Ok(keyring_key)
        }
        KeyringLookup::Missing => {
            if legacy_path.exists() {
                let legacy_key = read_legacy_master_key(&legacy_path)?;
                let legacy_b64 = STANDARD.encode(legacy_key);
                match backend.set(KEYRING_SERVICE, &account, &legacy_b64)? {
                    KeyringWriteOutcome::Stored => {
                        verify_keyring_persisted(backend, &account, &legacy_key)?;
                        if let Err(error) = std::fs::remove_file(&legacy_path) {
                            warn!(
                                path = %legacy_path.display(),
                                error = %error,
                                "master_key migrated to OS credential store, but deleting legacy file failed"
                            );
                        } else {
                            info!(account = %account, "migrated legacy plaintext master_key to OS credential store and removed plaintext file");
                        }
                        Ok(legacy_key)
                    }
                    KeyringWriteOutcome::Unavailable(reason) => {
                        if allow_file_fallback {
                            ensure_restrictive_file_permissions(&legacy_path);
                            warn!(reason = %reason, "OS credential store unavailable; continuing with restrictive master_key file fallback");
                            Ok(legacy_key)
                        } else {
                            Err(format!(
                                "OS credential store unavailable ({reason}); refusing to fall back to plaintext master_key file"
                            ))
                        }
                    }
                }
            } else {
                let key = generate_random_key();
                let key_b64 = STANDARD.encode(key);
                match backend.set(KEYRING_SERVICE, &account, &key_b64)? {
                    KeyringWriteOutcome::Stored => {
                        verify_keyring_persisted(backend, &account, &key)?;
                        Ok(key)
                    }
                    KeyringWriteOutcome::Unavailable(reason) => {
                        if allow_file_fallback {
                            warn!(reason = %reason, "OS credential store unavailable; storing master_key in restrictive fallback file");
                            write_fallback_master_key_file(&legacy_path, &key_b64)?;
                            Ok(key)
                        } else {
                            Err(format!(
                                "OS credential store unavailable ({reason}); refusing to create plaintext master_key file"
                            ))
                        }
                    }
                }
            }
        }
        KeyringLookup::Unavailable(reason) => {
            if !allow_file_fallback {
                return Err(format!(
                    "OS credential store unavailable ({reason}); refusing to fall back to plaintext master_key file"
                ));
            }
            if legacy_path.exists() {
                let key = read_legacy_master_key(&legacy_path)?;
                ensure_restrictive_file_permissions(&legacy_path);
                warn!(reason = %reason, "OS credential store unavailable; loaded restrictive fallback master_key file");
                Ok(key)
            } else {
                let key = generate_random_key();
                let key_b64 = STANDARD.encode(key);
                warn!(reason = %reason, "OS credential store unavailable; creating restrictive fallback master_key file");
                write_fallback_master_key_file(&legacy_path, &key_b64)?;
                Ok(key)
            }
        }
    }
}

/// AES-256-GCM 加密，前置 12 字节随机 Nonce
fn aes_encrypt(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key = get_master_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("AES key init failed: {}", e))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("AES encryption failed: {}", e))?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend(ciphertext);
    Ok(out)
}

/// AES-256-GCM 解密，提取前置 12 字节 Nonce
fn aes_decrypt(blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < NONCE_LEN {
        return Err("Encrypted payload too short".to_string());
    }
    let key = get_master_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("AES key init failed: {}", e))?;

    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES decryption failed: {}", e))
}

pub fn encrypt_provider_api_keys(settings: &mut AppSettings) -> Result<(), String> {
    for (provider, profile) in settings.provider_profiles.iter_mut() {
        let api_key = profile.api_key.trim();
        if api_key.is_empty()
            || is_encrypted_secret_value(api_key)
            || api_key.starts_with(LEGACY_DPAPI_PREFIX)
        {
            continue;
        }
        profile.api_key = encrypt_secret_value(api_key)
            .map_err(|error| format!("Failed to encrypt API key for {}: {}", provider, error))?;
    }
    Ok(())
}

pub fn decrypt_provider_api_keys(settings: &mut AppSettings) {
    for (provider, profile) in settings.provider_profiles.iter_mut() {
        let api_key = profile.api_key.trim();
        if api_key.is_empty() {
            continue;
        }
        if api_key.starts_with(SECRET_PREFIX) {
            match decrypt_secret_value(api_key) {
                Ok(plain) => profile.api_key = plain,
                Err(error) => {
                    warn!(
                        provider = %provider,
                        error = %error,
                        "failed to decrypt provider API key"
                    );
                    profile.api_key.clear();
                }
            }
            continue;
        }
        if api_key.starts_with(LEGACY_DPAPI_PREFIX) {
            warn!(
                provider = %provider,
                "legacy DPAPI-encrypted API key detected; please re-enter your API key"
            );
            profile.api_key.clear();
            continue;
        }
    }
}

pub fn has_plaintext_provider_api_keys(settings: &AppSettings) -> bool {
    settings.provider_profiles.values().any(|profile| {
        let api_key = profile.api_key.trim();
        !api_key.is_empty()
            && !is_encrypted_secret_value(api_key)
            && !api_key.starts_with(LEGACY_DPAPI_PREFIX)
    })
}

pub fn is_encrypted_secret_value(value: &str) -> bool {
    value.starts_with(SECRET_PREFIX)
}

pub fn encrypt_secret_value(value: &str) -> Result<String, String> {
    let protected = aes_encrypt(value.as_bytes())?;
    Ok(format!("{}{}", SECRET_PREFIX, STANDARD.encode(protected)))
}

pub fn decrypt_secret_value(value: &str) -> Result<String, String> {
    let encoded = value
        .strip_prefix(SECRET_PREFIX)
        .ok_or_else(|| "missing encrypted API key prefix".to_string())?;
    let protected = STANDARD
        .decode(encoded)
        .map_err(|error| format!("invalid encrypted API key payload: {}", error))?;
    let plain = aes_decrypt(&protected)?;
    String::from_utf8(plain).map_err(|error| format!("decrypted API key is not UTF-8: {}", error))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct MemoryKeyringBackend {
        entries: RefCell<HashMap<(String, String), String>>,
        unavailable_reason: Option<String>,
    }

    impl MemoryKeyringBackend {
        fn available() -> Self {
            Self {
                entries: RefCell::new(HashMap::new()),
                unavailable_reason: None,
            }
        }

        fn unavailable(reason: &str) -> Self {
            Self {
                entries: RefCell::new(HashMap::new()),
                unavailable_reason: Some(reason.to_string()),
            }
        }
    }

    impl KeyringBackend for MemoryKeyringBackend {
        fn get(&self, service: &str, account: &str) -> Result<KeyringLookup, String> {
            if let Some(reason) = &self.unavailable_reason {
                return Ok(KeyringLookup::Unavailable(reason.clone()));
            }
            Ok(
                match self
                    .entries
                    .borrow()
                    .get(&(service.to_string(), account.to_string()))
                {
                    Some(secret) => KeyringLookup::Found(secret.clone()),
                    None => KeyringLookup::Missing,
                },
            )
        }

        fn set(
            &self,
            service: &str,
            account: &str,
            secret: &str,
        ) -> Result<KeyringWriteOutcome, String> {
            if let Some(reason) = &self.unavailable_reason {
                return Ok(KeyringWriteOutcome::Unavailable(reason.clone()));
            }
            self.entries
                .borrow_mut()
                .insert((service.to_string(), account.to_string()), secret.to_string());
            Ok(KeyringWriteOutcome::Stored)
        }
    }

    #[test]
    fn fresh_install_stores_master_key_only_in_keyring() {
        let dir = TempDir::new().unwrap();
        let backend = MemoryKeyringBackend::available();

        let created = load_or_create_master_key_with(dir.path(), &backend, false).unwrap();
        assert!(!dir.path().join(MASTER_KEY_FILENAME).exists());

        let reloaded = load_or_create_master_key_with(dir.path(), &backend, false).unwrap();
        assert_eq!(created, reloaded);
    }

    #[test]
    fn legacy_plaintext_master_key_migrates_and_deletes_file() {
        let dir = TempDir::new().unwrap();
        let backend = MemoryKeyringBackend::available();
        let legacy_key = [0x42_u8; KEY_LEN];
        std::fs::write(
            dir.path().join(MASTER_KEY_FILENAME),
            STANDARD.encode(legacy_key),
        )
        .unwrap();

        let loaded = load_or_create_master_key_with(dir.path(), &backend, false).unwrap();
        assert_eq!(loaded, legacy_key);
        assert!(
            !dir.path().join(MASTER_KEY_FILENAME).exists(),
            "legacy plaintext master_key must be deleted after migration"
        );

        let second_load = load_or_create_master_key_with(dir.path(), &backend, false).unwrap();
        assert_eq!(second_load, legacy_key);
    }

    #[test]
    fn unavailable_keyring_refuses_plaintext_without_explicit_fallback() {
        let dir = TempDir::new().unwrap();
        let backend = MemoryKeyringBackend::unavailable("no desktop secret service");

        let err = load_or_create_master_key_with(dir.path(), &backend, false).unwrap_err();
        assert!(err.contains("OS credential store unavailable"));
        assert!(!dir.path().join(MASTER_KEY_FILENAME).exists());
    }

    #[test]
    fn unavailable_keyring_allows_file_fallback_when_enabled() {
        let dir = TempDir::new().unwrap();
        let backend = MemoryKeyringBackend::unavailable("headless linux container");

        let created = load_or_create_master_key_with(dir.path(), &backend, true).unwrap();
        assert!(dir.path().join(MASTER_KEY_FILENAME).exists());

        let reloaded = load_or_create_master_key_with(dir.path(), &backend, true).unwrap();
        assert_eq!(created, reloaded);
    }

    #[test]
    fn account_id_isolates_distinct_data_dirs() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        assert_ne!(
            master_key_account(dir_a.path()),
            master_key_account(dir_b.path())
        );
    }

    #[test]
    fn legacy_dpapi_prefix_is_not_treated_as_plaintext_or_double_encrypted() {
        let mut settings = AppSettings::default();
        let legacy = format!("{LEGACY_DPAPI_PREFIX}AQAAANCMnd8BFdERjHoAwE");
        settings.provider_profiles.insert(
            "legacy".to_string(),
            nova_protocol::settings::ProviderProfile {
                display_name: "Legacy".to_string(),
                api_key: legacy.clone(),
                base_url: "https://api.example.com".to_string(),
                model: "gpt-4o".to_string(),
                ..Default::default()
            },
        );
        assert!(!has_plaintext_provider_api_keys(&settings));
        encrypt_provider_api_keys(&mut settings).expect("legacy dpapi key should be skipped cleanly");
        assert_eq!(
            settings.provider_profiles.get("legacy").map(|p| p.api_key.as_str()),
            Some(legacy.as_str())
        );
    }
}
