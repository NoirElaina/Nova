// 统一动态模型元数据与定价服务 (SSOT: Single Source of Truth)
// 包含 OpenRouter 模型元数据、上下文窗口、最大输出 token、模态支持以及计费费率。
//
// 数据源策略：
// 1. 本地动态缓存 {app_data_dir}/models_cache.json —— 软件启动时加载，或在设置页面手动点击刷新；
// 2. 后台自动拉取 —— 启动时若缓存不存在或超过 24 小时，后台自动从 https://openrouter.ai/api/v1/models 拉新；
// 3. 首次离线兜底 —— 若初次启动处于纯离线环境且无本地缓存，内置极简常用模型列表，保证系统永不崩溃。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use tauri::Manager;

pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 32_768;

const MODELS_API_URL: &str = "https://openrouter.ai/api/v1/models";
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
const CACHE_FILE_NAME: &str = "models_cache.json";

/// 初次启动纯离线时的轻量基线兜底，仅约 1KB，彻底摆脱 680KB 的静态 models.json 编译负担。
static BASELINE_FALLBACK_RAW: &str = r#"[
  {"id":"anthropic/claude-3.7-sonnet","name":"Claude 3.7 Sonnet","context_length":200000,"architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0.000003","completion":"0.000015","input_cache_read":"0.0000003"},"top_provider":{"max_completion_tokens":65536}},
  {"id":"anthropic/claude-3.5-sonnet","name":"Claude 3.5 Sonnet","context_length":200000,"architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0.000003","completion":"0.000015","input_cache_read":"0.0000003"},"top_provider":{"max_completion_tokens":8192}},
  {"id":"deepseek/deepseek-chat","name":"DeepSeek V3","context_length":64000,"architecture":{"input_modalities":["text"]},"pricing":{"prompt":"0.00000014","completion":"0.00000028","input_cache_read":"0.000000014"},"top_provider":{"max_completion_tokens":8192}},
  {"id":"deepseek/deepseek-r1","name":"DeepSeek R1","context_length":64000,"architecture":{"input_modalities":["text"]},"pricing":{"prompt":"0.00000055","completion":"0.00000219","input_cache_read":"0.00000014"},"top_provider":{"max_completion_tokens":8192}},
  {"id":"openai/gpt-4o","name":"GPT-4o","context_length":128000,"architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0.0000025","completion":"0.00001","input_cache_read":"0.00000125"},"top_provider":{"max_completion_tokens":16384}},
  {"id":"openai/gpt-4o-mini","name":"GPT-4o mini","context_length":128000,"architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0.00000015","completion":"0.0000006","input_cache_read":"0.000000075"},"top_provider":{"max_completion_tokens":16384}},
  {"id":"google/gemini-2.5-flash","name":"Gemini 2.5 Flash","context_length":1048576,"architecture":{"input_modalities":["text","image","video","audio"]},"pricing":{"prompt":"0.000000075","completion":"0.0000003"},"top_provider":{"max_completion_tokens":8192}},
  {"id":"google/gemini-2.5-pro","name":"Gemini 2.5 Pro","context_length":2097152,"architecture":{"input_modalities":["text","image","video","audio"]},"pricing":{"prompt":"0.00000125","completion":"0.000005"},"top_provider":{"max_completion_tokens":8192}},
  {"id":"qwen/qwen-2.5-72b-instruct","name":"Qwen 2.5 72B Instruct","context_length":131072,"architecture":{"input_modalities":["text"]},"pricing":{"prompt":"0.00000035","completion":"0.0000004"},"top_provider":{"max_completion_tokens":8192}}
]"#;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelCatalogEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub top_provider: Option<TopProvider>,
    #[serde(default)]
    pub architecture: Option<Architecture>,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TopProvider {
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Architecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ModelPricing {
    pub prompt: String,
    pub completion: String,
    #[serde(default)]
    pub input_cache_read: Option<String>,
    #[serde(default)]
    pub input_cache_write: Option<String>,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelCatalogEntry>,
}

/// 前端展示的模型属性摘要（OpenRouter 风格）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub context_length: u64,
    pub max_completion_tokens: Option<u64>,
    pub input_modalities: Vec<String>,
    pub prompt_price_per_m: String,
    pub completion_price_per_m: String,
    pub cache_read_price_per_m: Option<String>,
}

/// 模型数据库状态统计。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogStats {
    pub total_models: usize,
    pub last_updated_secs: Option<u64>,
    pub cache_file_exists: bool,
    pub cache_file_path: String,
}

static RUNTIME_MODELS: RwLock<Option<Arc<Vec<ModelCatalogEntry>>>> = RwLock::new(None);
static BASELINE_ARC: OnceLock<Arc<Vec<ModelCatalogEntry>>> = OnceLock::new();

fn baseline_fallback_models() -> Arc<Vec<ModelCatalogEntry>> {
    BASELINE_ARC
        .get_or_init(|| {
            let list = serde_json::from_str::<Vec<ModelCatalogEntry>>(BASELINE_FALLBACK_RAW)
                .unwrap_or_default();
            Arc::new(list)
        })
        .clone()
}

pub fn effective_models() -> Arc<Vec<ModelCatalogEntry>> {
    if let Ok(guard) = RUNTIME_MODELS.read() {
        if let Some(runtime) = guard.clone() {
            return runtime;
        }
    }
    baseline_fallback_models()
}

/// 按模型标识查找条目（大小写不敏感，支持包含前缀的完整 ID 与后缀 slug 匹配）。
pub fn find_entry(model: &str) -> Option<ModelCatalogEntry> {
    let key = model.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let list = effective_models();
    list.iter()
        .find(|e| {
            let id = e.id.trim().to_ascii_lowercase();
            id == key || id.rsplit('/').next().is_some_and(|s| s == key)
        })
        .cloned()
}

/// 查询模型的上下文窗口 tokens；未命中返回 DEFAULT_CONTEXT_WINDOW。
pub fn get_context_window_tokens(model: &str) -> u32 {
    find_entry(model)
        .and_then(|e| e.context_length)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// 用户覆盖表查找（精确 -> 忽略大小写）。
pub fn lookup_user_override(model: &str, overrides: &HashMap<String, u32>) -> Option<u32> {
    let key = model.trim();
    if key.is_empty() {
        return None;
    }
    if let Some(&value) = overrides.get(key) {
        if value > 0 {
            return Some(value);
        }
    }
    let lower = key.to_ascii_lowercase();
    for (candidate, &value) in overrides {
        if value > 0 && candidate.trim().eq_ignore_ascii_case(&lower) {
            return Some(value);
        }
    }
    None
}

/// 解析最终上下文窗口：用户设置 > 内置/在线 models.json > 默认 200K。
pub fn resolve_context_window_tokens(model: &str, overrides: &HashMap<String, u32>) -> u32 {
    lookup_user_override(model, overrides).unwrap_or_else(|| get_context_window_tokens(model))
}

/// 查询模型的最大输出 token 数。
pub fn get_max_output_tokens(model: &str) -> u32 {
    find_entry(model)
        .and_then(|e| e.top_provider)
        .and_then(|tp| tp.max_completion_tokens)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

/// 查询模型是否支持图片输入。
pub fn supports_image_input(model: &str) -> bool {
    match find_entry(model) {
        Some(entry) => entry
            .architecture
            .map(|arch| arch.input_modalities.iter().any(|m| m.eq_ignore_ascii_case("image")))
            .unwrap_or(true),
        None => true,
    }
}

/// 获取模型定价，返回匹配的模型 ID 与 Pricing。
pub fn get_model_pricing(model: &str) -> Option<(String, ModelPricing)> {
    let entry = find_entry(model)?;
    let pricing = entry.pricing?;
    Some((entry.id, pricing))
}

fn format_price_per_m(raw: &str) -> String {
    if let Ok(val) = raw.trim().parse::<f64>() {
        let per_m = val * 1_000_000.0;
        if per_m == 0.0 {
            "Free".to_string()
        } else if per_m < 0.01 {
            format!("${:.4}", per_m)
        } else {
            format!("${:.2}", per_m)
        }
    } else {
        format!("${raw}")
    }
}

/// 查询模型列表（供设置界面展示与检索）。
pub fn list_models(search: Option<&str>, modality: Option<&str>) -> Vec<ModelCatalogSummary> {
    let list = effective_models();
    let query = search.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
    let mod_filter = modality.map(|m| m.trim().to_ascii_lowercase()).unwrap_or_default();

    list.iter()
        .filter(|entry| {
            if !query.is_empty() {
                let id_match = entry.id.to_ascii_lowercase().contains(&query);
                let name_match = entry
                    .name
                    .as_deref()
                    .map(|n| n.to_ascii_lowercase().contains(&query))
                    .unwrap_or(false);
                if !id_match && !name_match {
                    return false;
                }
            }

            if !mod_filter.is_empty() && mod_filter != "all" {
                let has_mod = entry
                    .architecture
                    .as_ref()
                    .map(|a| a.input_modalities.iter().any(|m| m.to_ascii_lowercase() == mod_filter))
                    .unwrap_or(false);
                if !has_mod {
                    return false;
                }
            }

            true
        })
        .map(|entry| {
            let name = entry.name.clone().unwrap_or_else(|| entry.id.clone());
            let context_length = entry.context_length.unwrap_or(DEFAULT_CONTEXT_WINDOW as u64);
            let max_completion_tokens = entry.top_provider.as_ref().and_then(|tp| tp.max_completion_tokens);
            let input_modalities = entry
                .architecture
                .as_ref()
                .map(|a| a.input_modalities.clone())
                .unwrap_or_else(|| vec!["text".to_string()]);
            let prompt_price_per_m = entry
                .pricing
                .as_ref()
                .map(|p| format_price_per_m(&p.prompt))
                .unwrap_or_else(|| "-".to_string());
            let completion_price_per_m = entry
                .pricing
                .as_ref()
                .map(|p| format_price_per_m(&p.completion))
                .unwrap_or_else(|| "-".to_string());
            let cache_read_price_per_m = entry
                .pricing
                .as_ref()
                .and_then(|p| p.input_cache_read.as_deref().map(format_price_per_m));

            ModelCatalogSummary {
                id: entry.id.clone(),
                name,
                description: entry.description.clone(),
                context_length,
                max_completion_tokens,
                input_modalities,
                prompt_price_per_m,
                completion_price_per_m,
                cache_read_price_per_m,
            }
        })
        .collect()
}

/// 获取当前缓存文件状态信息。
pub fn get_catalog_stats(app: &tauri::AppHandle) -> ModelCatalogStats {
    let (cache_path_str, exists, mtime_secs) = app
        .path()
        .app_data_dir()
        .map(|data_dir| {
            let p = data_dir.join(CACHE_FILE_NAME);
            let exists = p.is_file();
            let mtime = std::fs::metadata(&p)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            (p.display().to_string(), exists, mtime)
        })
        .unwrap_or_else(|_| (CACHE_FILE_NAME.to_string(), false, None));

    let total_models = effective_models().len();

    ModelCatalogStats {
        total_models,
        last_updated_secs: mtime_secs,
        cache_file_exists: exists,
        cache_file_path: cache_path_str,
    }
}

/// 立即从 OpenRouter 刷新模型列表并写入缓存。
pub async fn force_refresh(app: &tauri::AppHandle) -> Result<ModelCatalogStats, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(&data_dir);
    let cache_path = data_dir.join(CACHE_FILE_NAME);

    let text = fetch_models().await?;
    let list = parse_model_list(&text)
        .ok_or_else(|| "OpenRouter 返回数据格式无效或列表为空".to_string())?;

    let count = list.len();
    std::fs::write(&cache_path, &text).map_err(|e| format!("写入缓存文件失败: {e}"))?;
    set_runtime_models(list);
    tracing::info!(count, path = %cache_path.display(), "模型列表成功更新并存入缓存");

    Ok(get_catalog_stats(app))
}

/// 启动时初始化：加载已有本地缓存并启动后台自动刷新机制。
pub fn init(app: &tauri::AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        tracing::warn!("models cache disabled: app data dir unavailable");
        return;
    };
    let cache_path = data_dir.join(CACHE_FILE_NAME);

    // 同步加载已有缓存
    let mut cache_loaded = false;
    if let Ok(text) = std::fs::read_to_string(&cache_path) {
        if let Some(list) = parse_model_list(&text) {
            let count = list.len();
            set_runtime_models(list);
            cache_loaded = true;
            tracing::info!(count, path = %cache_path.display(), "models cache loaded");
        } else {
            let _ = std::fs::remove_file(&cache_path);
        }
    }

    // 若无本地缓存，初始时为快速给用户呈现数据，后台立即触发一次拉取
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if !cache_loaded || cache_stale(&cache_path) {
            let _ = force_refresh(&app_handle).await;
        }

        loop {
            tokio::time::sleep(CACHE_TTL).await;
            if cache_stale(&cache_path) {
                let _ = force_refresh(&app_handle).await;
            }
        }
    });
}

fn cache_stale(path: &std::path::Path) -> bool {
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(modified) => match modified.elapsed() {
            Ok(age) => age >= CACHE_TTL,
            Err(_) => true,
        },
        Err(_) => true,
    }
}

async fn fetch_models() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(MODELS_API_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("OpenRouter API returned HTTP {}", response.status()));
    }
    response.text().await.map_err(|e| e.to_string())
}

fn parse_model_list(text: &str) -> Option<Vec<ModelCatalogEntry>> {
    let list = serde_json::from_str::<ModelList>(text).ok()?;
    if list.data.is_empty() {
        return None;
    }
    Some(list.data)
}

fn set_runtime_models(list: Vec<ModelCatalogEntry>) {
    if let Ok(mut guard) = RUNTIME_MODELS.write() {
        *guard = Some(Arc::new(list));
    }
}
