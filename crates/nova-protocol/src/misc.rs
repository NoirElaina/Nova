//! 其余 IPC 线类型（配置目录/统计/任务等），与内核实现解耦。

use serde::{Deserialize, Serialize};

// ── 智能体套件 ──

/// 前端智能体配置页展示的可配置工具（勾选清单用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurableTool {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    /// 内核流程控制工具（不可移除）。
    pub always_on: bool,
}

/// 智能体来源：手动创建 / 市场安装 / 导入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentSourceKind {
    #[default]
    Manual,
    Market,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentSource {
    #[serde(default)]
    pub kind: AgentSourceKind,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundle {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 附加提示词（markdown）。空 = 不追加。
    #[serde(default)]
    pub prompt: String,
    /// 工具清单。None = 全部；Some = 仅勾选的（流程控制工具恒可用）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// 技能清单。None = 全部已启用技能；Some = 仅勾选的。
    #[serde(default)]
    pub enabled_skills: Option<Vec<String>>,
    /// MCP server 引用清单（全局定义，仅引用不存储）。None = 全部；Some = 仅勾选的。
    #[serde(default)]
    pub enabled_mcp_servers: Option<Vec<String>>,
    /// 全局启用开关：禁用后不可被新会话挂载。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 来源标注（手动 / 市场 / 导入）。
    #[serde(default)]
    pub source: AgentSource,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_true() -> bool {
    true
}

/// 流程控制工具：任何 bundle 下恒可见，不可被勾选移除。
pub const ALWAYS_ON_TOOLS: &[&str] = &["ask_user_question", "WritePlan"];
/// 仅默认 Nova 可用的工具：专用智能体不允许写全局记忆，防止领域会话污染跨会话记忆。
pub const DEFAULT_ONLY_TOOLS: &[&str] = &["memory"];

/// 归一化名字用于清单匹配（工具/技能/server 名大小写不敏感）。
fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

impl AgentBundle {
    /// 内置工具是否对该 bundle 可见（流程控制工具恒可见；默认专属工具恒不可见）。
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        if DEFAULT_ONLY_TOOLS.contains(&tool_name) {
            return false;
        }
        if ALWAYS_ON_TOOLS.contains(&tool_name) {
            return true;
        }
        match &self.enabled_tools {
            None => true,
            Some(allowed) => {
                allowed.iter().any(|n| normalize_name(n) == normalize_name(tool_name))
            }
        }
    }

    /// 技能是否对该 bundle 可见。
    pub fn is_skill_enabled(&self, skill_name: &str) -> bool {
        match &self.enabled_skills {
            None => true,
            Some(allowed) => {
                allowed.iter().any(|n| normalize_name(n) == normalize_name(skill_name))
            }
        }
    }

    /// MCP server 是否对该 bundle 可见。
    pub fn is_mcp_server_enabled(&self, server_name: &str) -> bool {
        match &self.enabled_mcp_servers {
            None => true,
            Some(allowed) => {
                allowed.iter().any(|n| normalize_name(n) == normalize_name(server_name))
            }
        }
    }
}

/// 智能体资料文件条目（agents/<id>/files/ 顶层）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileEntry {
    pub name: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

// ── 权限规则 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Allow,
    Deny,
}

/// 一条持久化权限规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    pub kind: RuleKind,
    /// 操作签名（与审批描述符 signature 同一归一化规则）。
    pub signature: String,
    pub created_at_ms: u64,
}

// ── 技能目录 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: String,
}

// ── 模型目录 ──

/// 前端展示的模型属性摘要（OpenRouter 风格）。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogStats {
    pub total_models: usize,
    pub last_updated_secs: Option<u64>,
    pub cache_file_exists: bool,
    pub cache_file_path: String,
}

// ── 终端探测 ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    /// 唯一标识，如 "pwsh", "powershell", "git-bash", "cmd", "wsl", "zsh", "bash", "fish"
    pub id: String,
    /// 显示名称，如 "PowerShell 7", "Windows PowerShell", "Git Bash"
    pub name: String,
    /// 可执行文件绝对路径
    pub path: String,
    /// 本机是否存在该终端
    pub is_available: bool,
    /// 是否为推荐默认项
    pub is_recommended: bool,
    /// 终端类型分类："powershell" | "cmd" | "bash" | "other"
    pub shell_type: String,
}

// ── Token 用量统计 ──

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub total_sessions: i64,
    pub total_messages: i64,
    pub total_tokens: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: String,
    pub active_days: i64,
    pub current_streak: i64,
    pub peak_hour: Option<i64>,
    pub favorite_model: Option<String>,
    pub heatmap: Vec<HeatmapPoint>,
    pub model_breakdown: Vec<ModelBreakdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapPoint {
    pub date: String,
    pub tokens: i64,
    pub sessions: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBreakdown {
    pub model: String,
    pub tokens: i64,
    pub calls: i64,
    pub cost_usd: String,
}

// ── 待办 / 定时任务 ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TodoEntry {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub recurring: bool,
    pub durable: bool,
    pub created_at: String,
}

// ── 模型拉取 / Git 初始化 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitGitRepoResult {
    /// true 表示这次调用新建了 `.git`；false 表示仓库已存在，本次为空操作。
    pub created: bool,
    pub path: String,
}

// ── Token 用量流水与会话聚合 ──

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageRecord {
    pub id: i64,
    pub conversation_id: Option<String>,
    pub model: String,
    pub provider: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<String>,
    pub source: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationUsageSummary {
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: String,
}

// ── 工作区 diff / git 状态 ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffLine {
    pub kind: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    pub path: String,
    pub absolute_path: String,
    pub change_type: String,
    pub additions: usize,
    pub deletions: usize,
    pub diff: Vec<FileDiffLine>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiff {
    pub files: Vec<WorkspaceFileChange>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepoStatus {
    pub initialized: bool,
    pub path: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

// ── 会话文件 / Shell 会话 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFileMeta {
    /// 文件名（已 sanitize，不含路径分隔符），同时也是读取时的唯一 key。
    pub filename: String,
    /// 文件大小（字节）。
    pub size: u64,
    /// 创建时间（Unix 秒）。
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSessionStatus {
    pub exists: bool,
    pub alive: bool,
    pub busy: bool,
    pub cwd: Option<String>,
    pub background_pids: Vec<u32>,
    pub background_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundOutput {
    pub id: String,
    pub pid: u32,
    pub command: String,
    pub cwd: Option<String>,
    pub running: bool,
    pub finished: bool,
    pub exit_code: Option<i32>,
    pub killed: bool,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub elapsed_ms: u64,
    pub ttl_ms: u64,
    pub remaining_ms: u64,
}

// ── 工作区浏览 ──

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub kind: String,
    pub extension: Option<String>,
    pub size: Option<u64>,
    pub modified: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDirectoryListing {
    pub root: String,
    pub path: String,
    pub relative_path: String,
    pub entries: Vec<WorkspaceEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileContent {
    pub path: String,
    pub relative_path: String,
    pub content: String,
    pub size: u64,
}

// ── 计划 / 回合轨迹 / 压缩 ──

/// 会话 plan 元信息，返回给前端展示。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPlan {
    /// plan 的 Markdown 全文。
    pub content: String,
    /// plan 文件更新时间（Unix 秒）。
    pub updated_at: i64,
}

/// 单个回合的轨迹：仅回合标识与最后一次模型收发（调试面板只展开发送/接收）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTrace {
    pub turn_id: String,
    /// 回合在事件流中的序号（排序/定位用）。
    pub seq: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub stop_reason: Option<String>,
    /// 本回合发给模型 API 的 wire 级请求/响应（按顺序配对）。
    pub wire_calls: Vec<WireCallTrace>,
}

/// 单次模型 API 请求及其响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCallTrace {
    /// 请求报文完整 JSON（含 system prompt/tools/消息数组）。
    pub request: serde_json::Value,
    /// 流结束后的完整响应 JSON（内容块/文本/stop_reason/用量）。
    pub response: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManualCompactOutcome {
    pub before_tokens: u32,
    pub after_tokens: u32,
    pub saved_tokens: u32,
    pub summary: String,
}

// ── 纯路径显示工具（桌面与内核共用） ──

pub use crate::path_display::{display_path_string, display_path_text};
