// 全项目唯一的 token 计算实现：OpenAI o200k_base BPE 分词器。
// 所有 token 估算（compact 决策、请求体、前端命令）一律走这里，不做模型/协议分支；
// provider 返回真实 usage 后由调用方覆盖，估算值仅作发送前的临时值。

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::llm::types::Message;

// 图片 token 公式（Anthropic 官方）：ceil(宽×高/750)，钳制在 [256, 8192]。
const IMAGE_TOKEN_PIXEL_DIVISOR: u64 = 750;
const IMAGE_TOKEN_MIN: i64 = 256;
const IMAGE_TOKEN_MAX: i64 = 8192;
// 图片尺寸无法解析时的保守值。
const IMAGE_TOKEN_FALLBACK: i64 = 1536;

// 进程内只加载一次词表；o200k_base 是现代模型（GPT-4o/5、o 系列）的通用基准编码。
static BPE: Lazy<tiktoken_rs::CoreBPE> =
    Lazy::new(|| tiktoken_rs::o200k_base().expect("failed to load o200k_base tokenizer"));

/// 预热词表；app 启动时后台调用一次，避免首次 token 计算时的加载开销。
pub fn warmup() {
    Lazy::force(&BPE);
}

/// 纯文本 token 数。
pub fn count_text(text: &str) -> i64 {
    BPE.encode_ordinary(text).len() as i64
}

/// 按目标 token 预算截断文本（二分字符数，保证结果 token 数不超过预算）。
pub fn truncate_to_token_budget(text: &str, budget: i64) -> String {
    if budget <= 0 || count_text(text) <= budget {
        return text.to_string();
    }

    // 不变量：前 lo 个字符的 token 数 <= budget；lo 即最终保留长度。
    let total_chars = text.chars().count();
    let mut lo = 0usize;
    let mut hi = total_chars;
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let prefix: String = text.chars().take(mid).collect();
        if count_text(&prefix) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    text.chars().take(lo).collect()
}

// 图片 token：按像素面积估算（Anthropic 官方公式）。
fn image_tokens_from_dimensions(width: u32, height: u32) -> i64 {
    if width == 0 || height == 0 {
        return IMAGE_TOKEN_FALLBACK;
    }
    let area = u64::from(width).saturating_mul(u64::from(height));
    (area.div_ceil(IMAGE_TOKEN_PIXEL_DIVISOR) as i64).clamp(IMAGE_TOKEN_MIN, IMAGE_TOKEN_MAX)
}

fn image_tokens_from_bytes(bytes: &[u8]) -> Option<i64> {
    let image = screenshots::image::load_from_memory(bytes).ok()?;
    Some(image_tokens_from_dimensions(image.width(), image.height()))
}

fn image_tokens_from_base64(data: &str) -> Option<i64> {
    let bytes = BASE64.decode(data.trim()).ok()?;
    image_tokens_from_bytes(&bytes)
}

// data:image/...;base64,... URL 按图片估算，避免 base64 文本炸穿上下文预测。
fn image_tokens_from_data_url(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    let lower = trimmed.get(..trimmed.len().min(32))?.to_ascii_lowercase();
    if !lower.starts_with("data:image/") {
        return None;
    }
    let (_, data) = trimmed.split_once(";base64,")?;
    image_tokens_from_base64(data).or(Some(IMAGE_TOKEN_FALLBACK))
}

// base64 图片 JSON 对象（如 {"type":"base64","media_type":"image/png","data":...}）。
fn image_tokens_from_json_object(map: &Map<String, Value>) -> Option<i64> {
    let media_type = map
        .get("media_type")
        .or_else(|| map.get("mime_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let source_type = map
        .get("type")
        .or_else(|| map.get("source_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let data = map.get("data").and_then(Value::as_str)?.trim();
    if data.is_empty() || (!source_type.eq("base64") && !media_type.starts_with("image/")) {
        return None;
    }
    Some(image_tokens_from_base64(data).unwrap_or(IMAGE_TOKEN_FALLBACK))
}

/// JSON 值 token 数：文本走 BPE，图片按像素公式，容器结构符号按固定开销计入。
pub fn count_json(value: &Value) -> i64 {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => count_text(&value.to_string()),
        Value::String(s) => image_tokens_from_data_url(s).unwrap_or_else(|| count_text(s) + 1),
        // 数组：头尾符号 + 每项递归 + 项分隔符。
        Value::Array(items) => 2 + items.iter().map(count_json).sum::<i64>() + items.len() as i64,
        Value::Object(map) => {
            if let Some(tokens) = image_tokens_from_json_object(map) {
                return tokens;
            }
            // 对象：头尾符号 + key 文本 + value 递归 + 每项分隔符。
            3 + map
                .iter()
                .map(|(k, v)| count_text(k) + count_json(v) + 2)
                .sum::<i64>()
        }
    }
}

/// 消息列表 token 数：消息序列化为内部 JSON 后整包计数，
/// role/块结构等开销自然包含在 JSON 文本中，与请求体估算同一条路径。
pub fn count_messages(messages: &[Message]) -> i64 {
    serde_json::to_value(messages)
        .map(|value| count_json(&value))
        .unwrap_or_default()
}

/// 任意可序列化值（如 provider 请求体）的 token 估算。
pub fn estimate_tokens_for_serializable<T: Serialize>(value: &T) -> Result<i64, String> {
    let value = serde_json::to_value(value)
        .map_err(|error| format!("failed to serialize value for token estimate: {error}"))?;
    Ok(count_json(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_empty_text_as_zero() {
        assert_eq!(count_text(""), 0);
    }

    #[test]
    fn counts_single_ascii_char_as_one_token() {
        assert_eq!(count_text("a"), 1);
    }

    #[test]
    fn cjk_text_is_never_underestimated() {
        // o200k 会合并常见中文词组（如"你好"），但密度仍远高于英文：
        // 每 token 平均不超过 2 个 CJK 字符，绝不会被字符数÷4式的算法低估。
        let text = "你好世界这是一个测试";
        assert!(count_text(text) * 2 >= text.chars().count() as i64);
    }

    #[test]
    fn mixed_text_is_more_than_english_only_ratio() {
        // 中文 token 密度显著高于英文（英文约 4 字符/token）。
        let cjk = count_text("你好世界这是一个测试");
        let latin = count_text("hello world this is a test");
        assert!(cjk * 2 > latin);
    }

    #[test]
    fn json_structure_adds_overhead_over_plain_text() {
        let plain = "hello";
        let wrapped = json!({ "content": "hello" });
        assert!(count_json(&wrapped) > count_text(plain));
    }

    #[test]
    fn messages_cost_more_than_their_plain_text() {
        let messages = vec![Message {
            role: crate::llm::types::Role::User,
            content: crate::llm::types::Content::Text("hello world".to_string()),
        }];
        assert!(count_messages(&messages) > count_text("hello world"));
    }

    #[test]
    fn image_data_url_is_not_counted_as_base64_text() {
        // 1x1 PNG base64：按像素公式应得到 IMAGE_TOKEN_MIN，
        // 而不是按 base64 文本长度数出几十上百个 token。
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        assert_eq!(count_json(&Value::String(url.to_string())), IMAGE_TOKEN_MIN);
    }

    #[test]
    fn image_source_object_is_counted_by_pixels() {
        // ContentBlock::Image 序列化后的 source 对象：像素公式而非 base64 长度。
        let source = json!({
            "type": "base64",
            "media_type": "image/png",
            "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
        });
        assert_eq!(count_json(&source), IMAGE_TOKEN_MIN);
    }

    #[test]
    fn truncate_respects_token_budget() {
        let text = "你好世界".repeat(100);
        let budget = 50_i64;
        let truncated = truncate_to_token_budget(&text, budget);
        assert!(count_text(&truncated) <= budget);
        assert!(truncated.chars().count() < text.chars().count());
    }

    #[test]
    fn truncate_keeps_short_text_intact() {
        let text = "short text";
        assert_eq!(truncate_to_token_budget(text, 100), text);
    }
}
