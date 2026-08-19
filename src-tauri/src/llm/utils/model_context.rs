// 从 windowTokens/models.json 中读取模型的上下文窗口大小。
// JSON 格式为 OpenRouter 模型列表（数组，每个元素有 id / context_length / top_provider.max_completion_tokens）。
//
// 两级数据源：
// 1. 编译期嵌入的 windowTokens/models.json —— 离线兜底，永不失效；
// 2. 运行时缓存 {app_data_dir}/models_cache.json —— init() 启动时加载，
//    超过 24 小时自动从 https://openrouter.ai/api/v1/models 拉新（后台任务），
//    新模型无需等发版即可获得正确的上下文窗口。
// 用户可在设置里为任意模型覆盖上下文窗口（优先于两级 JSON / 默认值）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use serde::Deserialize;
use tauri::Manager;

#[derive(Debug, Clone, Deserialize)]
struct ModelEntry {
    /// 模型 id，如 "xiaomi/mimo-v2.5-pro"。
    id: String,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    top_provider: TopProvider,
    #[serde(default)]
    architecture: Architecture,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TopProvider {
    #[serde(default)]
    max_completion_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Architecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;
// 默认最大输出 token。工程任务（多文件改动+规划+多工具调用）需要更长的输出空间，
// 8K 会截断复杂规划，32K 是工程 agent 的合理默认值。
// 模型自身的 max_completion_tokens 若更小则会被 find_entry 覆盖。
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 32_768;

static MODEL_DB_RAW: &str = include_str!("../../windowTokens/models.json");

/// OpenRouter 模型列表 API（免费、无需鉴权）。
const MODELS_API_URL: &str = "https://openrouter.ai/api/v1/models";
/// 缓存有效期：超过此时长后台拉新。
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// 运行时缓存文件名（位于 app_data_dir）。
const CACHE_FILE_NAME: &str = "models_cache.json";

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

/// 运行时缓存路径；init() 成功后可用。
static CACHE_PATH: OnceLock<PathBuf> = OnceLock::new();
/// 运行时模型表（下载/加载成功后存在）；查询优先于编译期嵌入表。
static RUNTIME_MODELS: RwLock<Option<Arc<Vec<ModelEntry>>>> = RwLock::new(None);

fn embedded_models() -> &'static Vec<ModelEntry> {
    static LIST: OnceLock<Vec<ModelEntry>> = OnceLock::new();
    LIST.get_or_init(|| {
        serde_json::from_str::<ModelList>(MODEL_DB_RAW)
            .map(|l| l.data)
            .unwrap_or_default()
    })
}

/// 查询当前生效的模型表：运行时缓存 > 编译期嵌入。
fn effective_models() -> Arc<Vec<ModelEntry>> {
    if let Ok(guard) = RUNTIME_MODELS.read() {
        if let Some(runtime) = guard.clone() {
            return runtime;
        }
    }
    Arc::clone(EMBEDDED_ARC.get_or_init(|| Arc::new(embedded_models().clone())))
}

/// 编译期嵌入表的 Arc 共享（与运行时表统一为 Arc<Vec<_>>）。
static EMBEDDED_ARC: OnceLock<Arc<Vec<ModelEntry>>> = OnceLock::new();

/// 按名字查找 JSON 条目（返回克隆，条目很小，开销可忽略）。
/// 匹配规则：id 的最后一段（'/' 右侧）与 model 参数相等，大小写不敏感。
/// 例如 "mimo-v2.5-pro" 可以命中 id="xiaomi/mimo-v2.5-pro"。
fn find_entry(model: &str) -> Option<ModelEntry> {
    let key = model.trim().to_ascii_lowercase();
    let list = effective_models();
    list.iter().find(|e| {
        let id = e.id.trim().to_ascii_lowercase();
        // 先完整匹配，再匹配 '/' 后的 slug
        id == key || id.rsplit('/').next().map_or(false, |s| s == key)
    }).cloned()
}

/// 查询内置 JSON 中的上下文窗口；未命中返回 DEFAULT_CONTEXT_WINDOW。
pub fn get_context_window_tokens(model: &str) -> u32 {
    find_entry(model)
        .and_then(|e| e.context_length)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// 用户覆盖表查找（精确 → 忽略大小写）。
fn lookup_user_override(model: &str, overrides: &HashMap<String, u32>) -> Option<u32> {
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

/// 解析最终上下文窗口：用户设置 > 内置 models.json > 默认 200K。
/// 新模型未进 JSON 时，只要用户在设置里配了就不会被压到默认值。
pub fn resolve_context_window_tokens(model: &str, overrides: &HashMap<String, u32>) -> u32 {
    lookup_user_override(model, overrides).unwrap_or_else(|| get_context_window_tokens(model))
}

/// 查询模型的最大输出 token 数。
pub fn get_max_output_tokens(model: &str) -> u32 {
    find_entry(model)
        .and_then(|e| e.top_provider.max_completion_tokens)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

/// 查询模型是否支持图片输入。
/// 模型不在预置库中时乐观返回 true（假装支持，让 API 自己决定）。
pub fn supports_image_input(model: &str) -> bool {
    match find_entry(model) {
        Some(entry) => entry
            .architecture
            .input_modalities
            .iter()
            .any(|m| m.eq_ignore_ascii_case("image")),
        None => true,
    }
}

/// 启动时初始化：登记缓存路径、加载已有缓存、启动每 24h 的后台刷新任务。
/// 在 app setup 阶段调用一次；失败不影响启动（回落编译期嵌入表）。
pub fn init(app: &tauri::AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        tracing::warn!("models cache disabled: app data dir unavailable");
        return;
    };
    let cache_path = data_dir.join(CACHE_FILE_NAME);
    let _ = CACHE_PATH.set(cache_path.clone());

    // 同步加载已有缓存（无论新旧都先用着，总比编译期嵌入的新）。
    match std::fs::read_to_string(&cache_path) {
        Ok(text) => match parse_model_list(&text) {
            Some(list) => {
                let count = list.len();
                set_runtime_models(list);
                tracing::info!(count, path = %cache_path.display(), "models cache loaded");
            }
            None => {
                // 缓存损坏（半截写入等），删掉让后台任务重拉。
                let _ = std::fs::remove_file(&cache_path);
            }
        },
        Err(_) => {} // 无缓存，正常首启
    }

    // 后台任务：启动时若缓存过期立即刷新；之后每 24h 检查一次。
    tauri::async_runtime::spawn(async move {
        loop {
            if cache_stale(&cache_path) {
                match fetch_models().await {
                    Ok(text) => match parse_model_list(&text) {
                        Some(list) => {
                            let count = list.len();
                            set_runtime_models(list);
                            if let Err(error) = std::fs::write(&cache_path, &text) {
                                tracing::warn!(error = %error, "failed to write models cache");
                            } else {
                                tracing::info!(count, "models cache refreshed from OpenRouter");
                            }
                        }
                        None => tracing::warn!("OpenRouter models response failed validation"),
                    },
                    Err(error) => {
                        // 网络失败不致命：保留现有缓存/嵌入表，24h 后再试。
                        tracing::warn!(error = %error, "models cache refresh failed");
                    }
                }
            }
            tokio::time::sleep(CACHE_TTL).await;
        }
    });
}

/// 缓存是否需要刷新：文件不存在，或修改时间早于 TTL 前。
fn cache_stale(path: &std::path::Path) -> bool {
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(modified) => match modified.elapsed() {
            Ok(age) => age >= CACHE_TTL,
            Err(_) => true, // mtime 在未来（时钟跳变），视为过期
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
        return Err(format!("OpenRouter API returned {}", response.status()));
    }
    response.text().await.map_err(|e| e.to_string())
}

/// 解析并校验模型列表：格式正确且非空才接受，防止半截响应顶掉好数据。
fn parse_model_list(text: &str) -> Option<Vec<ModelEntry>> {
    let list = serde_json::from_str::<ModelList>(text).ok()?;
    if list.data.is_empty() {
        return None;
    }
    Some(list.data)
}

fn set_runtime_models(list: Vec<ModelEntry>) {
    if let Ok(mut guard) = RUNTIME_MODELS.write() {
        *guard = Some(Arc::new(list));
    }
}
