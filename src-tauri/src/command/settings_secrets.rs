use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::RngCore;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};
use tracing::warn;

use crate::command::settings::AppSettings;

/// ?????????
const SECRET_PREFIX: &str = "nova:v2:";
/// ? DPAPI ?????????????????????
const LEGACY_DPAPI_PREFIX: &str = "nova:dpapi:v1:";
/// ???????
const MASTER_KEY_FILENAME: &str = "master_key";
/// AES-256-GCM nonce ???12 ?? / 96 ???
const NONCE_LEN: usize = 12;
/// AES-256 ?????32 ?? / 256 ???
const KEY_LEN: usize = 32;

/// ?????????????????????
static MASTER_KEY: OnceLock<[u8; KEY_LEN]> = OnceLock::new();

/// ???????????????????????????
pub fn init_master_key(app: &AppHandle) -> Result<(), String> {
    let key = load_or_create_master_key(app)?;
    MASTER_KEY
        .set(key)
        .map_err(|_| "Master key already initialized".to_string())
}

/// ??????????????? init_master_key?
fn get_master_key() -> Result<[u8; KEY_LEN], String> {
    MASTER_KEY
        .get()
        .copied()
        .ok_or_else(|| "Master key not initialized; call init_master_key first".to_string())
}

/// ? app data ????????????????????
fn load_or_create_master_key(app: &AppHandle) -> Result<[u8; KEY_LEN], String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;
    let key_path = data_dir.join(MASTER_KEY_FILENAME);

    if key_path.exists() {
        let key_b64 = std::fs::read_to_string(&key_path)
            .map_err(|e| format!("Failed to read master key file: {}", e))?;
        let key_bytes = STANDARD
            .decode(key_b64.trim())
            .map_err(|e| format!("Corrupt master key file: {}", e))?;
        let key: [u8; KEY_LEN] = key_bytes
            .try_into()
            .map_err(|_| "Master key file is wrong size".to_string())?;
        return Ok(key);
    }

    // ?????????????????
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create app data directory: {}", e))?;

    let mut key = [0u8; KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut key);
    let key_b64 = STANDARD.encode(key);
    std::fs::write(&key_path, &key_b64)
        .map_err(|e| format!("Failed to write master key file: {}", e))?;
    Ok(key)
}

/// AES-256-GCM ???
/// ?????nonce(12 bytes) + ciphertext + tag??? base64 ???
fn aes_encrypt(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key = get_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("AES key init failed: {}", e))?;

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

/// AES-256-GCM ???
/// ?????nonce(12 bytes) + ciphertext + tag??? base64 ???
fn aes_decrypt(blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < NONCE_LEN {
        return Err("Encrypted payload too short".to_string());
    }
    let key = get_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("AES key init failed: {}", e))?;

    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES decryption failed: {}", e))
}

pub fn encrypt_provider_api_keys(settings: &mut AppSettings) -> Result<(), String> {
    for (provider, profile) in settings.provider_profiles.iter_mut() {
        let api_key = profile.api_key.trim();
        if api_key.is_empty() || is_encrypted_secret_value(api_key) {
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
        !api_key.is_empty() && !is_encrypted_secret_value(api_key)
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
    String::from_utf8(plain)
        .map_err(|error| format!("decrypted API key is not UTF-8: {}", error))
}
