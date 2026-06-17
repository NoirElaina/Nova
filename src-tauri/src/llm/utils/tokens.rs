use once_cell::sync::Lazy;
use tiktoken_rs::cl100k_base;

// 使用 Lazy 避免每次计算时重复加载词表（加载词表有一定开销）
static BPE: Lazy<tiktoken_rs::CoreBPE> = Lazy::new(|| {
    cl100k_base().expect("Failed to load cl100k_base tokenizer")
});

pub fn tokenCountWithEstimation(messages: &[crate::llm::types::Message]) -> usize {
    // 使用 tiktoken-rs 提供的真实分词器来计算 token 数量
    // 采用 cl100k_base 词表 (GPT-4 / Claude 主流词表兼容)
    messages.iter().map(|m| {
        BPE.encode_with_special_tokens(&m.content.text).len()
    }).sum()
}
















