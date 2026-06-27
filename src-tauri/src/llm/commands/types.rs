use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMeta {
    // 会话唯一 ID。
    pub id: String,
    // 会话标题。
    pub title: String,
    // 最近更新时间（unix 秒）。
    pub updated_at: i64,
    // 置顶时间（unix 秒），为空表示未置顶。
    pub pinned_at: Option<i64>,
    // 该会话绑定的项目工作区目录绝对路径，为空表示用内置默认工作区。
    pub workspace_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistoryAttachment {
    // 附件显示名称。
    pub source_name: String,
    // 可选 mime 类型。
    pub mime_type: Option<String>,
    // 可选文件大小（字节）。
    pub size: Option<u64>,
    // 附件类型（document/image）。
    pub kind: Option<String>,
    // 图片媒体类型（仅 image 附件）。
    pub media_type: Option<String>,
    // 图片 base64 数据（仅 image 附件）。
    pub data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMessage {
    // 消息角色（user/assistant）。
    pub role: String,
    // 消息文本内容。
    pub content: String,
    // 可选思考内容。
    pub reasoning: Option<String>,
    // 可选附件列表。
    pub attachments: Option<Vec<HistoryAttachment>>,
    // 可选 token 使用量。
    pub token_usage: Option<i64>,
    // 可选成本结构（JSON）。
    pub cost: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistoryToolExecution {
    // 工具调用唯一 ID（会话内唯一）。
    pub id: String,
    // 所属对话回合 ID（旧记录可能为空）。
    pub turn_id: Option<String>,
    // 工具名称。
    pub tool_name: String,
    // 工具输入参数（JSON 文本或纯文本）。
    pub input: String,
    // 工具执行结果。
    pub result: String,
    // 执行状态（running/completed/error/cancelled）。
    pub status: String,
    // 开始时间（unix 毫秒）。
    pub started_at: i64,
    // 结束时间（unix 毫秒，可选）。
    pub finished_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMemory {
    // 会话摘要。
    pub summary: String,
    // 关键事实列表。
    pub key_facts: Vec<String>,
    // 记忆更新时间（unix 秒）。
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConversationHandover {
    // 会话 ID。
    pub conversation_id: String,
    // 会话标题。
    pub title: String,
    // 摘要文本。
    pub summary: String,
    // 关键事实。
    pub key_facts: Vec<String>,
    // 最近消息列表。
    pub recent_messages: Vec<HistoryMessage>,
    // 被省略消息数。
    pub omitted_message_count: i64,
    // 总消息数。
    pub total_message_count: i64,
    // 更新时间（unix 秒）。
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompactContext {
    // 会话 ID。
    pub conversation_id: String,
    // 压缩上下文文本。
    pub context_text: String,
    // 采用的 recent limit。
    pub recent_limit: i64,
    // 被省略消息数。
    pub omitted_message_count: i64,
    // 总消息数。
    pub total_message_count: i64,
    // 估算 token 数。
    pub estimated_tokens: i64,
    // 更新时间（unix 秒）。
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompactBoundary {
    // 边界记录 ID。
    pub id: i64,
    // 会话 ID。
    pub conversation_id: String,
    // 该次 compact 的文本上下文。
    pub context_text: String,
    // compact 摘要。
    pub summary: String,
    // compact 关键事实。
    pub key_facts: Vec<String>,
    // recent limit。
    pub recent_limit: i64,
    // 被省略消息数。
    pub omitted_message_count: i64,
    // 总消息数。
    pub total_message_count: i64,
    // 估算 token 数。
    pub estimated_tokens: i64,
    // 创建时间（unix 秒）。
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResumeContext {
    // 恢复基线边界。
    pub boundary: CompactBoundary,
    // 边界之后的消息列表。
    pub messages_since_boundary: Vec<HistoryMessage>,
}
