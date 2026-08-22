//! 工具权限裁决：正交化的审批策略模型。
//!
//! 三层结构：
//! 1. 描述符（descriptors/checks）：操作是什么、风险多高（Safe/Risky/Forbidden）；
//! 2. 持久化规则（rules）：用户"始终允许/拒绝"的跨会话决定，命令类支持前缀匹配；
//! 3. 审批策略（ApprovalPolicy）：AlwaysAsk / OnRequest / Never，Auto 模式当轮覆盖为 Never。
//!
//! 裁决顺序：硬拒绝（Forbidden）→ 持久规则 → 会话允许集 → 策略裁决 → AskUser。

pub mod checks;
pub mod descriptors;
pub mod rules;

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::oneshot;

use checks::{assess_mcp_operation, truncate_chars};

pub use checks::protected_path_violation;
pub(crate) use descriptors::{describe_file_write_permission, describe_shell_command_permission};

const DEFAULT_PERMISSION_SCOPE: &str = "__global__";
const PENDING_APPROVAL_TTL_MS: u64 = 15 * 60 * 1000;
const ACTION_TOKEN_TTL_MS: u64 = 60 * 60 * 1000;

/// 操作风险级别：由描述符声明，描述事实而非裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 低风险（只读命令、普通路径写入）：OnRequest 策略下直接放行。
    Safe,
    /// 需要关注（非白名单命令、敏感路径读取、无法静态分析）：交由策略裁决。
    Risky,
    /// 硬性禁止（受保护路径、空命令）：任何策略下都拒绝。
    Forbidden,
}

/// 审批策略：与操作风险正交的全局开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// 凡产生权限描述符的操作一律审批（Forbidden 仍直接拒绝）。
    AlwaysAsk,
    /// 仅 Risky 操作审批，Safe 直接放行。
    #[default]
    OnRequest,
    /// 从不审批：Risky 也放行（Forbidden 仍直接拒绝）。
    /// Auto 迭代模式当轮等效于此策略。
    Never,
}

impl ApprovalPolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "always_ask" | "alwaysask" => Some(Self::AlwaysAsk),
            "on_request" | "onrequest" => Some(Self::OnRequest),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    pub fn as_setting_str(&self) -> &'static str {
        match self {
            Self::AlwaysAsk => "always_ask",
            Self::OnRequest => "on_request",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Clone)]
struct ProtectedOperation {
    signature: String,
    preview: String,
    warning: Option<String>,
    risk: RiskLevel,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    operation: ProtectedOperation,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Copy)]
struct RecordedDecision {
    action: PermissionAction,
    decided_at_ms: u64,
}

#[derive(Debug, Default)]
struct ConversationPermissionState {
    pending: HashMap<String, PendingApproval>,
    pending_by_signature: HashMap<String, String>,
    allow_once: HashSet<String>,
    allow_session: HashSet<String>,
    resolved_by_request: HashMap<String, RecordedDecision>,
}

#[derive(Debug, Default)]
struct PermissionState {
    conversations: HashMap<String, ConversationPermissionState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    /// 始终允许：写入持久化规则，跨会话生效。
    AllowAlways,
    DenyOnce,
}

#[derive(Debug)]
pub enum PermissionEnforcement {
    Allow,
    Deny(String),
    AskUser { request_id: String, payload: String },
}

fn permission_waiters() -> &'static Mutex<HashMap<String, oneshot::Sender<PermissionAction>>> {
    static WAITERS: OnceLock<Mutex<HashMap<String, oneshot::Sender<PermissionAction>>>> =
        OnceLock::new();
    WAITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_permission_waiter(request_id: &str) -> oneshot::Receiver<PermissionAction> {
    let (tx, rx) = oneshot::channel();
    if let Ok(mut guard) = permission_waiters().lock() {
        guard.insert(request_id.to_string(), tx);
    }
    rx
}

fn unregister_permission_waiter(request_id: &str) {
    if let Ok(mut guard) = permission_waiters().lock() {
        guard.remove(request_id);
    }
}

fn notify_permission_waiter(request_id: &str, action: PermissionAction) {
    if let Ok(mut guard) = permission_waiters().lock() {
        if let Some(sender) = guard.remove(request_id) {
            let _ = sender.send(action);
        }
    }
}

fn permission_state() -> &'static Mutex<PermissionState> {
    static STATE: OnceLock<Mutex<PermissionState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PermissionState::default()))
}

fn next_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        // 权限过期只需要毫秒精度，u64 足够表达。
        .map(|d| d.as_millis() as u64)
        // 系统时钟异常（早于 epoch）时回退到 0，避免 panic。
        .unwrap_or(0)
}

fn conversation_scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        // 将 conversation_id 裁剪为没有前后空白的值。
        .map(str::trim)
        // 空字符串视为未提供会话 id。
        .filter(|id| !id.is_empty())
        // 子代理会话（`{parent}:sub:{uuid}`）归一化到父会话：
        // 会话级 allow 等决策随父会话生效，
        // 避免子代理因 scope 不一致而行为分裂。
        .map(crate::llm::services::subagent::parent_conversation_id)
        // 缺省落到全局 scope。
        .unwrap_or(DEFAULT_PERMISSION_SCOPE)
        .to_string()
}

fn conversation_state_mut<'a>(
    state: &'a mut PermissionState,
    conversation_id: Option<&str>,
) -> &'a mut ConversationPermissionState {
    // Keep all permission decisions scoped by conversation, with a shared fallback scope.
    let scope = conversation_scope_key(conversation_id);
    // scope: 当前会话或全局 permission scope。
    state.conversations.entry(scope).or_default()
}

fn prune_expired_pending(state: &mut ConversationPermissionState) {
    // Expire old pending approvals to prevent stale request ids from being reused.
    let now = now_millis();
    // now: 当前时间毫秒。
    let mut expired_request_ids = Vec::new();
    // expired_request_ids: 需要删除的过期请求 id 列表。

    for (request_id, pending) in &state.pending {
        // request_id: pending map 的键；pending: 待审批数据。
        // saturating_sub 防止时钟回拨导致下溢。
        if now.saturating_sub(pending.created_at_ms) > PENDING_APPROVAL_TTL_MS {
            expired_request_ids.push(request_id.clone());
        }
    }

    for request_id in expired_request_ids {
        // request_id: 即将过期的待审批请求 id。
        // 两张索引表都要清理，避免 signature 指向已删除请求。
        if let Some(pending) = state.pending.remove(&request_id) {
            state
                .pending_by_signature
                .remove(&pending.operation.signature);
            notify_permission_waiter(&request_id, PermissionAction::DenyOnce);
        }
    }
}

fn prune_resolved_decisions(state: &mut ConversationPermissionState) {
    let now = now_millis();
    state
        .resolved_by_request
        .retain(|_, record| now.saturating_sub(record.decided_at_ms) <= ACTION_TOKEN_TTL_MS);
}

fn upsert_pending_request_id(
    state: &mut ConversationPermissionState,
    operation: &ProtectedOperation,
) -> String {
    // Reuse an existing pending request for the same operation signature when possible.
    if let Some(existing_id) = state
        .pending_by_signature
        .get(&operation.signature)
        .cloned()
    {
        // existing_id: 已记录的 request id。
        // signature -> request_id 命中且 request 仍在 pending，直接复用。
        if state.pending.contains_key(&existing_id) {
            return existing_id;
        }
        // 索引命中但主体缺失，说明是脏索引，先清掉再重建。
        state.pending_by_signature.remove(&operation.signature);
    }

    let request_id = next_request_id();
    // request_id: 新生成的审批请求 id。
    state.pending.insert(
        request_id.clone(),
        PendingApproval {
            operation: operation.clone(),
            created_at_ms: now_millis(),
        },
    );
    state
        .pending_by_signature
        .insert(operation.signature.clone(), request_id.clone());
    request_id
}

fn operation_from_input(app: &AppHandle, tool_name: &str, input: &Value) -> Option<ProtectedOperation> {
    // 内置工具读取自己显式声明的权限描述；
    // 不做按 tool_name 的隐式兜底，避免策略散落在权限模块里。
    if let Some(descriptor) =
        crate::llm::tools::permission_descriptor_for_tool(tool_name, input)
    {
        return Some(ProtectedOperation {
            signature: descriptor.signature,
            preview: descriptor.preview,
            warning: descriptor.warning,
            risk: descriptor.risk,
        });
    }

    if let Some((server, tool)) = crate::llm::services::mcp_tools::parse_mcp_tool_name(tool_name) {
        // 对 MCP 动态工具做基于 server/tool 名和参数的统一风险推断。
        let (risk, warning, signature) = assess_mcp_operation(tool_name, &server, &tool, input);
        return Some(ProtectedOperation {
            signature,
            preview: format!("{} {}", tool_name, truncate_chars(&input.to_string(), 160)),
            warning,
            risk,
        });
    }

    // 未声明权限描述的工具：确认设置可读后即视为不受控（与描述符缺失语义一致）。
    let _ = app;
    None
}

/// 有效审批策略：当轮覆盖（Auto 模式 → Never）优先于全局设置。
pub fn effective_approval_policy(
    app: &AppHandle,
    policy_override: Option<ApprovalPolicy>,
) -> ApprovalPolicy {
    if let Some(policy) = policy_override {
        return policy;
    }
    match crate::command::settings::load_settings(app) {
        Ok(settings) => ApprovalPolicy::parse(&settings.approval_policy).unwrap_or_default(),
        Err(_) => ApprovalPolicy::default(),
    }
}

fn build_permission_prompt_payload(operation: &ProtectedOperation) -> String {
    let mut context = format!("请求执行高风险操作：{}", operation.preview);
    // context: 用户审批提示上下文。
    if let Some(w) = &operation.warning {
        // w: 风险提示文本。
        // 把规则命中的风险信息拼进上下文，便于用户做授权决策。
        context.push_str("。风险提示：");
        context.push_str(&humanize_permission_warning(w));
    }

    json!({
        "type": "needs_user_input",
        "context": context,
        "allow_freeform": true,
        "questions": [
            {
                "header": "权限审批",
                "question": "请选择处理方式",
                "multi_select": false,
                "options": [
                    {
                        "label": "仅本次允许",
                        "value": "allow_once",
                        "description": "只放行这一次，执行后自动失效"
                    },
                    {
                        "label": "本会话允许",
                        "value": "allow_session",
                        "description": "本次应用运行期间对同一操作持续放行"
                    },
                    {
                        "label": "始终允许",
                        "value": "allow_always",
                        "description": "记住该操作（命令类含同前缀命令），以后不再询问"
                    },
                    {
                        "label": "拒绝",
                        "value": "deny_once",
                        "description": "只拒绝这一次"
                    }
                ]
            }
        ]
    })
    .to_string()
}

fn extract_single_quoted(raw: &str) -> Option<String> {
    let start = raw.find('\'')?;
    let remain = &raw[start + 1..];
    let end = remain.find('\'')?;
    Some(remain[..end].to_string())
}

fn humanize_permission_warning(raw: &str) -> String {
    let stripped = raw
        .trim()
        .trim_start_matches("Blocked by permission gate: ")
        .to_string();

    if stripped.contains("命令为空") || stripped.contains("command is empty") {
        return "命令为空，已被安全策略拦截。".to_string();
    }

    if stripped.contains("target path is empty") {
        return "目标路径为空，已被安全策略拦截。".to_string();
    }

    // AST 解析相关：无法静态分析
    if stripped.contains("无法静态分析") || stripped.contains("解析器不可用") {
        return format!("{}。", stripped);
    }

    // 语义检查相关：eval-like builtin、危险 builtin 等
    if stripped.contains("需要确认") || stripped.contains("需要审批") {
        return format!("{}。", stripped);
    }

    // 路径约束
    if stripped.contains("命中受保护路径") || stripped.contains("writing protected path") {
        if let Some(path) = extract_single_quoted(&stripped) {
            return format!("目标路径 '{}' 属于受保护目录，已拦截。", path);
        }
        return "目标路径属于受保护目录，已拦截。".to_string();
    }

    if stripped.contains("命中敏感路径") || stripped.contains("writing sensitive path") {
        if let Some(path) = extract_single_quoted(&stripped) {
            return format!("目标路径 '{}' 属于敏感目录，已拦截。", path);
        }
        return "目标路径属于敏感目录，已拦截。".to_string();
    }

    format!("{}。", stripped)
}

pub fn parse_permission_action_name(action: &str) -> Option<PermissionAction> {
    match action.trim().to_ascii_lowercase().as_str() {
        "allow_once" => Some(PermissionAction::AllowOnce),
        "allow_session" => Some(PermissionAction::AllowSession),
        "allow_always" => Some(PermissionAction::AllowAlways),
        "deny_once" => Some(PermissionAction::DenyOnce),
        _ => None,
    }
}

fn apply_decision(
    app: &AppHandle,
    state: &mut ConversationPermissionState,
    action: PermissionAction,
    request_id: &str,
) -> bool {
    let Some(pending) = state.pending.remove(request_id) else {
        return false;
    };
    // pending: 找到的待审批请求，如果不存在则说明该 token 已失效。

    let signature = pending.operation.signature;
    // signature: 该操作的唯一归一化签名。
    state.pending_by_signature.remove(&signature);
    // 先移除旧决策，确保同一 signature 在允许集合里互斥。
    state.allow_once.remove(&signature);
    state.allow_session.remove(&signature);

    match action {
        PermissionAction::AllowOnce => {
            state.allow_once.insert(signature.clone());
        }
        PermissionAction::AllowSession => {
            state.allow_session.insert(signature.clone());
        }
        PermissionAction::AllowAlways => {
            // 会话内立即生效 + 持久化规则跨会话生效。
            state.allow_session.insert(signature.clone());
            if let Err(error) = rules::add_rule(app, rules::RuleKind::Allow, &signature) {
                tracing::warn!(error = %error, signature = %signature, "failed to persist allow rule");
            }
        }
        PermissionAction::DenyOnce => {
            // 一次性拒绝：不记忆，仅通知等待方本次拒绝。
        }
    }

    state.resolved_by_request.insert(
        request_id.to_string(),
        RecordedDecision {
            action,
            decided_at_ms: now_millis(),
        },
    );
    notify_permission_waiter(request_id, action);

    true
}

pub fn submit_permission_decision(
    app: &AppHandle,
    conversation_id: Option<&str>,
    request_id: &str,
    action: PermissionAction,
) -> Result<bool, String> {
    let mut guard = permission_state()
        .lock()
        .map_err(|_| "Permission state unavailable due to lock poisoning".to_string())?;
    let state = conversation_state_mut(&mut guard, conversation_id);
    prune_expired_pending(state);
    prune_resolved_decisions(state);

    if apply_decision(app, state, action, request_id) {
        return Ok(true);
    }

    Ok(state.resolved_by_request.contains_key(request_id))
}

pub async fn await_permission_decision(
    conversation_id: Option<&str>,
    request_id: &str,
    timeout_ms: u64,
) -> Result<PermissionAction, String> {
    let conversation_scope = conversation_id.map(|v| v.to_string());

    {
        let mut guard = permission_state()
            .lock()
            .map_err(|_| "Permission state unavailable due to lock poisoning".to_string())?;
        let state = conversation_state_mut(&mut guard, conversation_scope.as_deref());
        prune_expired_pending(state);
        prune_resolved_decisions(state);

        if let Some(record) = state.resolved_by_request.get(request_id) {
            return Ok(record.action);
        }

        if !state.pending.contains_key(request_id) {
            return Err(format!(
                "Permission request '{}' is no longer pending",
                request_id
            ));
        }
    }

    let mut receiver = register_permission_waiter(request_id);
    let started_at = now_millis();
    let timeout_ms = timeout_ms.max(1);

    loop {
        tokio::select! {
            recv = &mut receiver => {
                return recv.map_err(|_| {
                    "Permission waiter closed before decision was received".to_string()
                });
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {}
        }

        if crate::llm::cancellation::is_cancelled(conversation_scope.as_deref()) {
            unregister_permission_waiter(request_id);
            return Err("Permission approval cancelled".to_string());
        }

        if now_millis().saturating_sub(started_at) > timeout_ms {
            unregister_permission_waiter(request_id);
            return Err("Permission approval timed out".to_string());
        }

        let resolved = {
            let mut guard = permission_state()
                .lock()
                .map_err(|_| "Permission state unavailable due to lock poisoning".to_string())?;
            let state = conversation_state_mut(&mut guard, conversation_scope.as_deref());
            prune_expired_pending(state);
            prune_resolved_decisions(state);
            state
                .resolved_by_request
                .get(request_id)
                .copied()
                .map(|r| r.action)
        };

        if let Some(action) = resolved {
            unregister_permission_waiter(request_id);
            return Ok(action);
        }
    }
}

pub fn enforce_tool_permission(
    app: &AppHandle,
    conversation_id: Option<&str>,
    policy_override: Option<ApprovalPolicy>,
    tool_name: &str,
    input: &Value,
) -> PermissionEnforcement {
    let Some(operation) = operation_from_input(app, tool_name, input) else {
        // operation: None 表示该工具没有声明受控操作，权限层不参与拦截。
        return PermissionEnforcement::Allow;
    };
    // operation: 当前待评估的受控操作。

    // 1. 硬拒绝：Forbidden 操作在任何策略下都不放行。
    if operation.risk == RiskLevel::Forbidden {
        let reason = operation
            .warning
            .clone()
            .unwrap_or_else(|| format!("Operation '{}' is forbidden by security policy", tool_name));
        return PermissionEnforcement::Deny(humanize_permission_warning(&reason));
    }

    // 2. 持久化规则：跨会话记住的允许/拒绝决定（命令类含前缀匹配）。
    let persisted = rules::load_rules(app);
    if let Some(rule) = rules::find_matching_rule(&persisted, &operation.signature) {
        return match rule.kind {
            rules::RuleKind::Allow => PermissionEnforcement::Allow,
            rules::RuleKind::Deny => PermissionEnforcement::Deny(format!(
                "Operation '{}' is blocked by a persisted deny rule",
                tool_name
            )),
        };
    }

    // 3. 会话内决策：本会话允许 / 仅本次允许。
    {
        let mut guard = match permission_state().lock() {
            Ok(g) => g,
            Err(_) => {
                return PermissionEnforcement::Deny(
                    "Permission state unavailable due to lock poisoning".to_string(),
                )
            }
        };
        // guard: 全局 permission state 的锁引用。

        let state = conversation_state_mut(&mut guard, conversation_id);
        // state: 当前 conversation 的权限状态。
        prune_expired_pending(state);
        prune_resolved_decisions(state);

        if state.allow_session.contains(&operation.signature) {
            // 会话级允许可重复使用。
            return PermissionEnforcement::Allow;
        }

        if state.allow_once.remove(&operation.signature) {
            // 一次性允许命中后立即消费，确保只生效一次。
            return PermissionEnforcement::Allow;
        }
    }

    // 4. 策略裁决：Auto 模式当轮覆盖为 Never。
    let policy = effective_approval_policy(app, policy_override);
    let needs_ask = match policy {
        ApprovalPolicy::Never => false,
        ApprovalPolicy::OnRequest => operation.risk == RiskLevel::Risky,
        ApprovalPolicy::AlwaysAsk => true,
    };

    if !needs_ask {
        return PermissionEnforcement::Allow;
    }

    // 5. 子代理没有审批通道：审批请求会以 `:sub:` 派生会话 ID 发到前端，
    // 但该 ID 不是可切换的真实会话，用户永远无法处理，
    // 子代理会一直挂到审批超时（默认 15 分钟）才失败。
    // 这里直接拒绝并把原因告诉模型，让它换路径或把该操作交回主代理。
    if crate::llm::services::subagent::is_subagent_conversation(conversation_id) {
        return PermissionEnforcement::Deny(format!(
            "Permission required for tool '{}' in subagent context, but subagents cannot ask the user for approval. Use a different, non-sensitive path or report back so the parent agent can perform this operation.",
            tool_name
        ));
    }

    let mut guard = match permission_state().lock() {
        Ok(g) => g,
        Err(_) => {
            return PermissionEnforcement::Deny(
                "Permission state unavailable due to lock poisoning".to_string(),
            )
        }
    };
    let state = conversation_state_mut(&mut guard, conversation_id);
    let request_id = upsert_pending_request_id(state, &operation);
    // request_id: 生成或复用的待审批请求 id。
    PermissionEnforcement::AskUser {
        request_id,
        payload: build_permission_prompt_payload(&operation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_parse_roundtrip() {
        assert_eq!(
            ApprovalPolicy::parse("always_ask"),
            Some(ApprovalPolicy::AlwaysAsk)
        );
        assert_eq!(
            ApprovalPolicy::parse("ON_REQUEST"),
            Some(ApprovalPolicy::OnRequest)
        );
        assert_eq!(ApprovalPolicy::parse("never"), Some(ApprovalPolicy::Never));
        assert_eq!(ApprovalPolicy::parse("bogus"), None);
        assert_eq!(ApprovalPolicy::default(), ApprovalPolicy::OnRequest);
    }

    #[test]
    fn action_name_parsing() {
        assert!(matches!(
            parse_permission_action_name("allow_always"),
            Some(PermissionAction::AllowAlways)
        ));
        assert!(matches!(
            parse_permission_action_name("Allow_Session"),
            Some(PermissionAction::AllowSession)
        ));
        assert!(parse_permission_action_name("nope").is_none());
    }

    #[test]
    fn warning_humanized() {
        let out = humanize_permission_warning(
            "Blocked by permission gate: writing protected path 'c:/windows/x'.",
        );
        assert!(out.contains("受保护目录"));
    }
}
