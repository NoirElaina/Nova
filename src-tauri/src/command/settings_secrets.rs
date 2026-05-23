use base64::{engine::general_purpose::STANDARD, Engine as _};
use tracing::warn;

use crate::command::settings::AppSettings;

const SECRET_PREFIX: &str = "nova:dpapi:v1:";

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
        if !is_encrypted_secret_value(api_key) {
            continue;
        }
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
    let protected = platform::protect(value.as_bytes())?;
    Ok(format!("{}{}", SECRET_PREFIX, STANDARD.encode(protected)))
}

pub fn decrypt_secret_value(value: &str) -> Result<String, String> {
    let encoded = value
        .strip_prefix(SECRET_PREFIX)
        .ok_or_else(|| "missing encrypted API key prefix".to_string())?;
    let protected = STANDARD
        .decode(encoded)
        .map_err(|error| format!("invalid encrypted API key payload: {}", error))?;
    let plain = platform::unprotect(&protected)?;
    String::from_utf8(plain).map_err(|error| format!("decrypted API key is not UTF-8: {}", error))
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use std::slice;

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    const OPTIONAL_ENTROPY: &[u8] = b"nova.settings.provider_api_key.v1";

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "Crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *mut DataBlob,
            data_descr: *const u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            data_in: *mut DataBlob,
            data_descr: *mut *mut u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "Kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    pub fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
        crypt_data(bytes, true)
    }

    pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
        crypt_data(bytes, false)
    }

    fn crypt_data(bytes: &[u8], protect: bool) -> Result<Vec<u8>, String> {
        let mut input = blob_from_slice(bytes)?;
        let mut entropy = blob_from_slice(OPTIONAL_ENTROPY)?;
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: null_mut(),
        };

        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &mut input,
                    std::ptr::null(),
                    &mut entropy,
                    null_mut(),
                    null_mut(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &mut input,
                    null_mut(),
                    &mut entropy,
                    null_mut(),
                    null_mut(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            }
        };

        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }

        let data =
            unsafe { slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec();
        unsafe {
            LocalFree(output.pb_data.cast::<c_void>());
        }
        Ok(data)
    }

    fn blob_from_slice(bytes: &[u8]) -> Result<DataBlob, String> {
        let len =
            u32::try_from(bytes.len()).map_err(|_| "secret payload is too large".to_string())?;
        Ok(DataBlob {
            cb_data: len,
            pb_data: bytes.as_ptr() as *mut u8,
        })
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    pub fn protect(_bytes: &[u8]) -> Result<Vec<u8>, String> {
        Err("API key encryption currently requires Windows DPAPI".to_string())
    }

    pub fn unprotect(_bytes: &[u8]) -> Result<Vec<u8>, String> {
        Err("API key decryption currently requires Windows DPAPI".to_string())
    }
}
