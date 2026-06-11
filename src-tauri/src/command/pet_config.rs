use serde_json::Value;

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

    println!("Request URL: {}", url);

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