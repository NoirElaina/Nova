// 重新导出自统一服务 crate::services::model_catalog
// 保持原有调用路径的完全向后兼容。

pub use crate::services::model_catalog::{
    get_context_window_tokens, get_max_output_tokens, init, resolve_context_window_tokens,
    supports_image_input, DEFAULT_CONTEXT_WINDOW, DEFAULT_MAX_OUTPUT_TOKENS,
};
