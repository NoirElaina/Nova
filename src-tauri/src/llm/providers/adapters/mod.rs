use crate::llm::providers::stream_runner::Delta;
use crate::llm::types::{AgentMode, Message};
use reqwest::RequestBuilder;

pub mod anthropic;
pub mod openai;
pub(crate) mod reasoning;
pub mod responses;

/// 协议适配器：只负责格式转换，不持有网络连接
pub trait ApiAdapter: Send + Sync {
    /// 构造目标 API 的 HTTP 请求（包含头部和请求体）
    fn build_request(
        &mut self,
        builder: RequestBuilder,
        app: &tauri::AppHandle,
        messages: &[Message],
        agent_mode: AgentMode,
        conversation_id: Option<&str>,
    ) -> Result<RequestBuilder, String>;

    /// 解析一条 SSE event_data，返回通用 Delta 列表
    fn parse_event(&mut self, data: &str) -> Result<Vec<Delta>, String>;

    /// 流结束时的清理（如部分协议需要在结束后汇总工具调用）
    fn flush(&mut self) -> Vec<Delta> {
        Vec::new()
    }

    fn provider_name(&self) -> &'static str;
}
