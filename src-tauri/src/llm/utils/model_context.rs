// 从 windowTokens/models.json 中读取模型的上下文窗口大小。
// JSON 格式为 OpenRouter 模型列表（数组，每个元素有 id / context_length / top_provider.max_completion_tokens）。
// 文件在编译期嵌入，运行时懒解析一次，之后直接在 Vec 上按名字匹配。

use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ModelEntry {
    /// 模型 id，如 "xiaomi/mimo-v2.5-pro"。
    id: String,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    top_provider: TopProvider,
}

#[derive(Debug, Deserialize, Default)]
struct TopProvider {
    #[serde(default)]
    max_completion_tokens: Option<u64>,
}

pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8_192;

static MODEL_DB_RAW: &str = include_str!("../../windowTokens/models.json");

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

fn models() -> &'static Vec<ModelEntry> {
    static LIST: OnceLock<Vec<ModelEntry>> = OnceLock::new();
    LIST.get_or_init(|| {
        serde_json::from_str::<ModelList>(MODEL_DB_RAW)
            .map(|l| l.data)
            .unwrap_or_default()
    })
}

/// 按名字查找 JSON 条目。
/// 匹配规则：id 的最后一段（'/' 右侧）与 model 参数相等，大小写不敏感。
/// 例如 "mimo-v2.5-pro" 可以命中 id="xiaomi/mimo-v2.5-pro"。
fn find_entry(model: &str) -> Option<&'static ModelEntry> {
    let key = model.trim().to_ascii_lowercase();
    models().iter().find(|e| {
        let id = e.id.trim().to_ascii_lowercase();
        // 先完整匹配，再匹配 '/' 后的 slug
        id == key || id.rsplit('/').next().map_or(false, |s| s == key)
    })
}

/// 查询模型的输入上下文窗口大小（token 数）。
pub fn get_context_window_tokens(model: &str) -> u32 {
    find_entry(model)
        .and_then(|e| e.context_length)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// 查询模型的最大输出 token 数。
pub fn get_max_output_tokens(model: &str) -> u32 {
    find_entry(model)
        .and_then(|e| e.top_provider.max_completion_tokens)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}
