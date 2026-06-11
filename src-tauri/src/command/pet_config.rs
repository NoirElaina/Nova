use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use tauri::{AppHandle, Manager};
use zip::ZipArchive;

#[tauri::command]
pub async fn fetch_pet(
    page: u32,
    page_size: u32,
    sort: String,
    kind: Option<String>,
    tag: Option<String>,
) -> Result<Value, String> {
    let mut url = format!(
        "https://codex-pets.net/api/pets?page={}&pageSize={}&sort={}",
        page, page_size, sort
    );

    if let Some(kind) = kind {
        if !kind.is_empty() && kind != "all" {
            url.push_str(&format!("&kind={}", kind));
        }
    }

    if let Some(tag) = tag {
        if !tag.is_empty() && tag != "all" {
            url.push_str(&format!("&tag={}", tag));
        }
    }

    let response = reqwest::get(&url)
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;

    response
        .json::<Value>()
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize)]
pub struct LocalPetMeta {
    id: String,
    display_name: String,
    cell_size: String,
    atlas_size: String,
}

#[tauri::command]
pub async fn download_pet(
    app: AppHandle,
    pet_id: String,
    display_name: String,
    download_url: String,
    cell_size: String,
    atlas_size: String,
) -> Result<String, String> {
    let pet_dir = app
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("pet")
        .join(&pet_id);

    std::fs::create_dir_all(&pet_dir).map_err(|e| e.to_string())?;

    let full_url = if download_url.starts_with("http") {
        download_url
    } else {
        format!("https://codex-pets.net{}", download_url)
    };

    let response = reqwest::get(&full_url)
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("Invalid zip: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let entry_path = entry.mangled_name();
        let out_path = pet_dir.join(&entry_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut outfile = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut outfile, &buf).map_err(|e| e.to_string())?;
        }
    }

    let meta = LocalPetMeta {
        id: pet_id,
        display_name,
        cell_size,
        atlas_size,
    };
    let meta_json =
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    std::fs::write(pet_dir.join("pet.json"), meta_json).map_err(|e| e.to_string())?;

    Ok(pet_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn list_local_pets(app: AppHandle) -> Result<Vec<LocalPetMeta>, String> {
    let pet_root = app
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("pet");

    if !pet_root.exists() {
        return Ok(Vec::new());
    }

    let mut pets = Vec::new();
    let entries = std::fs::read_dir(&pet_root).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let meta_path = path.join("pet.json");
        if !meta_path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
        if let Ok(meta) = serde_json::from_str::<LocalPetMeta>(&content) {
            pets.push(meta);
        }
    }

    Ok(pets)
}

#[tauri::command]
pub async fn get_pet_spritesheet(app: AppHandle, pet_id: String) -> Result<String, String> {
    let pet_dir = app
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("pet")
        .join(&pet_id);

    let spritesheet_path = pet_dir.join("spritesheet.webp");
    if !spritesheet_path.exists() {
        return Err(format!("Spritesheet not found for pet: {pet_id}"));
    }

    let bytes = std::fs::read(&spritesheet_path).map_err(|e| e.to_string())?;
    let b64 = STANDARD.encode(&bytes);
    Ok(format!("data:image/webp;base64,{b64}"))
}
