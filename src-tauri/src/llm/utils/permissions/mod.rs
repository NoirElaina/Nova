use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::oneshot;

use crate::llm::tools::ToolPermissionDescriptor;

// Command fragments considered destructive enough to always be gated.
const DANGEROUS_COMMAND_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -rf /",
    "rm -rf /*",
    "del /f /s",
    "del /f /s /q",
    "remove-item -recurse",
    "remove-item -force",
    "remove-item -recurse -force",
    "format c:",
    "diskpart",
    "shutdown /s",
    "shutdown -h",
    "reboot",
    "mkfs",
    "git reset --hard",
    "git clean -fd",
    "git clean -fdx",
    // 删整个仓库元数据会毁掉审查/回退能力，始终拦截。
    "rm -rf .git",
    "rm -rf /.git",
    "rmdir /s /q .git",
    "remove-item -recurse -force .git",
];

// Path prefixes that should never be written without explicit override.
const PROTECTED_PATH_PREFIXES: &[&str] = &[
    "c:\\windows",
    "c:\\program files",
    "c:\\program files (x86)",
    "c:\\programdata",
    "c:\\users\\public",
    "/etc",
    "/bin",
    "/sbin",
    "/usr",
    "/var",
    "/boot",
    "/system",
];

// Sensitive path markers that should be blocked even outside protected roots.
const PROTECTED_PATH_CONTAINS: &[&str] = &[
    "\\.ssh\\",
    "/.ssh/",
    "\\.aws\\",
    "/.aws/",
    "\\.gnupg\\",
    "/.gnupg/",
    "\\.config\\git",
    "/.config/git",
    "\\.git\\config",
    "/.git/config",
    // 整个 .git 目录都受保护，任何写入删除都拦截。
    "\\.git\\",
    "/.git/",
];

const DEFAULT_PERMISSION_SCOPE: &str = "__global__";
const PENDING_APPROVAL_TTL_MS: u64 = 15 * 60 * 1000;
const ACTION_TOKEN_TTL_MS: u64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy)]
struct RecordedDecision {
    action: PermissionAction,
    decided_at_ms: u64,
}

fn unsafe_override_enabled() -> bool {
    std::env::var("NOVA_ALLOW_UNSAFE_TOOLS")
        .map(|v| {
            // 统一做 trim + 小写，避免环境变量大小写或空格导致误判。
            let normalized = v.trim().to_ascii_lowercase();
            // normalized: 规范化后的环境变量值。
            // 兼容常见的布尔开关写法。
            normalized == "1" || normalized == "true" || normalized == "yes" || normalized == "on"
        })
        // 变量缺失时默认关闭不安全放行。
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct ProtectedOperation {
    signature: String,
    preview: String,
    warning: Option<String>,
    needs_approval: bool,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    operation: ProtectedOperation,
    created_at_ms: u64,
}

#[derive(Debug, Default)]
struct ConversationPermissionState {
    pending: HashMap<String, PendingApproval>,
    pending_by_signature: HashMap<String, String>,
    allow_once: HashSet<String>,
    allow_session: HashSet<String>,
    deny_session: HashSet<String>,
    resolved_by_request: HashMap<String, RecordedDecision>,
}

#[derive(Debug, Default)]
struct PermissionState {
    conversations: HashMap<String, ConversationPermissionState>,
}

#[derive(Debug, Clone, Copy)]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    DenySession,
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
            notify_permission_waiter(&request_id, PermissionAction::DenySession);
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

fn normalize_path_for_match(path: &str) -> String {
    // 用统一分隔符与小写比较，减少跨平台路径写法差异。
    path.trim().replace('/', "\\").to_ascii_lowercase()
}

fn normalize_command_for_match(command: &str) -> String {
    command
        // 压缩空白，避免同义命令因空格差异得到不同签名。
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        // 统一小写，减少大小写差异干扰。
        .to_ascii_lowercase()
}

fn contains_shell_word(command: &str, target: &str) -> bool {
    command.split_whitespace().any(|token| {
        // token: 当前命令片段。
        // 去掉包裹在 token 两侧的标点，保留单词内部的 -/_。
        let cleaned =
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
        // cleaned: 去除边界标点后的纯命令单词。
        // 这里做“完整单词”比较，避免误伤如 "rmdir" 对 "rm" 的包含。
        cleaned == target
    })
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect::<String>()
}

fn looks_like_shell_mcp(server: &str, tool: &str) -> bool {
    let s = format!(
        "{} {}",
        server.to_ascii_lowercase(),
        tool.to_ascii_lowercase()
    );
    // s: server+tool 的小写拼接字符串。
    ["bash", "shell", "powershell", "pwsh", "terminal"]
        .iter()
        // 关键字模糊匹配：适配不同 MCP server/tool 命名习惯。
        .any(|k| s.contains(k))
}

fn looks_like_file_mcp(server: &str, tool: &str) -> bool {
    let s = format!(
        "{} {}",
        server.to_ascii_lowercase(),
        tool.to_ascii_lowercase()
    );
    // s: server+tool 的小写拼接字符串。
    ["file", "filesystem", "fs", "write", "edit", "replace"]
        .iter()
        // 关键字命中即按文件写操作风控处理。
        .any(|k| s.contains(k))
}

fn pick_string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        // key: 当前尝试提取的字段名。
        if let Some(v) = value.get(*key).and_then(|v| v.as_str()) {
            // v: JSON 字段值。
            let trimmed = v.trim();
            // trimmed: 去掉前后空白后的字符串。
            if !trimmed.is_empty() {
                // 返回原始 JSON 字符串切片，零拷贝。
                return Some(trimmed);
            }
        }
    }
    None
}

fn check_mcp_operation(server: &str, tool: &str, arguments: &Value) -> Result<(), String> {
    if looks_like_shell_mcp(server, tool) {
        // 兼容不同 server 的参数命名。
        let command =
            pick_string_field(arguments, &["command", "cmd", "script"]).unwrap_or_default();
        // command: shell 操作中提取到的命令字符串。
        return check_command(command);
    }

    if looks_like_file_mcp(server, tool) {
        // 常见路径参数别名统一提取。
        let path = pick_string_field(
            arguments,
            &["path", "file", "file_path", "target", "target_path"],
        )
        .unwrap_or_default();
        // path: 文件操作中提取到的目标路径。
        return check_file_path(path);
    }

    Ok(())
}

pub(crate) fn describe_shell_command_permission(
    tool_name: &str,
    preview_label: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    // command: 当前工具请求执行的终端命令文本。
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();

    if command.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:<empty>", tool_name),
            preview: "命令为空".to_string(),
            warning: Some("命令为空，无法执行。".to_string()),
            needs_approval: false,
        });
    }

    let normalized = normalize_command_for_match(command);
    // warning: 命令命中危险规则时返回给用户看的风险提示。
    let warning = check_command(command).err();

    Some(ToolPermissionDescriptor {
        signature: format!("{}:{}", tool_name, normalized),
        preview: format!(
            "{}（{}）：{}",
            preview_label,
            tool_name,
            truncate_chars(command, 180)
        ),
        warning: warning.clone(),
        needs_approval: warning.is_some(),
    })
}

pub(crate) fn describe_file_write_permission(
    tool_name: &str,
    preview_label: &str,
    path_key: &str,
    input: &Value,
) -> Option<ToolPermissionDescriptor> {
    // path: 当前写操作的目标路径；不同工具可通过 path_key 复用这个 helper。
    let path = input
        .get(path_key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();

    if path.is_empty() {
        return Some(ToolPermissionDescriptor {
            signature: format!("{}:<empty>", tool_name),
            preview: "路径为空".to_string(),
            warning: Some("目标路径为空，无法执行。".to_string()),
            needs_approval: false,
        });
    }

    let normalized = normalize_path_for_match(path);
    // warning: 路径命中受保护目录或敏感标记时生成风险提示。
    let warning = check_file_path(path).err();

    Some(ToolPermissionDescriptor {
        signature: format!("{}:{}", tool_name, normalized),
        preview: format!(
            "{}（{}）：{}",
            preview_label,
            tool_name,
            truncate_chars(path, 200)
        ),
        warning: warning.clone(),
        needs_approval: warning.is_some(),
    })
}

fn operation_from_input(tool_name: &str, input: &Value) -> Option<ProtectedOperation> {
    // 内置工具先读取自己显式声明的权限描述；
    // 这里不再按 tool_name 做隐式兜底，避免策略散落在权限模块里。
    if let Some(operation) = crate::llm::tools::permission_descriptor_for_tool(tool_name, input) {
        return Some(ProtectedOperation {
            signature: operation.signature,
            preview: operation.preview,
            warning: operation.warning,
            needs_approval: operation.needs_approval,
        });
    }

    if let Some((server, tool)) = crate::llm::services::mcp_tools::parse_mcp_tool_name(tool_name) {
        // risk: 对 MCP 动态工具做基于 server/tool 名和参数的统一风险推断。
        let risk = check_mcp_operation(&server, &tool, input).err();

        return Some(ProtectedOperation {
            signature: format!(
                "{}:{}:{}",
                tool_name,
                server.to_ascii_lowercase(),
                normalize_command_for_match(&input.to_string())
            ),
            preview: format!("{} {}", tool_name, truncate_chars(&input.to_string(), 160)),
            warning: risk.clone(),
            needs_approval: risk.is_some(),
        });
    }

    None
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
                        "label": "拒绝并记住",
                        "value": "deny_session",
                        "description": "本会话拒绝同一操作，直到会话结束"
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
        .replace(
            " Set NOVA_ALLOW_UNSAFE_TOOLS=1 only for trusted debugging.",
            "",
        );

    if stripped.contains("command is empty") {
        return "命令为空，已被安全策略拦截。".to_string();
    }

    if stripped.contains("target path is empty") {
        return "目标路径为空，已被安全策略拦截。".to_string();
    }

    if stripped.contains("command contains dangerous shell command") {
        if let Some(word) = extract_single_quoted(&stripped) {
            return format!("命令包含高危 shell 指令 '{}'，默认已拦截。", word);
        }
        return "命令包含高危 shell 指令，默认已拦截。".to_string();
    }

    if stripped.contains("command contains dangerous pattern") {
        if let Some(pattern) = extract_single_quoted(&stripped) {
            return format!("命令命中高危模式 '{}'，默认已拦截。", pattern);
        }
        return "命令命中高危模式，默认已拦截。".to_string();
    }

    if stripped.contains("writing protected path") {
        if let Some(path) = extract_single_quoted(&stripped) {
            return format!("目标路径 '{}' 属于受保护目录，已拦截。", path);
        }
        return "目标路径属于受保护目录，已拦截。".to_string();
    }

    if stripped.contains("writing sensitive path") {
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
        "deny_session" => Some(PermissionAction::DenySession),
        _ => None,
    }
}

fn apply_decision(
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
    // 先移除旧决策，确保同一 signature 在三种集合里互斥。
    state.allow_once.remove(&signature);
    state.allow_session.remove(&signature);
    state.deny_session.remove(&signature);

    match action {
        PermissionAction::AllowOnce => {
            state.allow_once.insert(signature);
        }
        PermissionAction::AllowSession => {
            state.allow_session.insert(signature);
        }
        PermissionAction::DenySession => {
            state.deny_session.insert(signature);
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

    if apply_decision(state, action, request_id) {
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

fn check_command(command: &str) -> Result<(), String> {
    let normalized = normalize_command_for_match(command);
    // normalized: 规范化后用于风险检测的命令文本。
    if normalized.is_empty() {
        return Err("Blocked by permission gate: command is empty".to_string());
    }

    for dangerous_word in ["rm", "del", "remove-item"] {
        // dangerous_word: 当前检查的危险关键字。
        // 先做单词级命中，拦截最常见的删除命令。
        if contains_shell_word(&normalized, dangerous_word) {
            return Err(format!(
                "Blocked by permission gate: command contains dangerous shell command '{}'. Set NOVA_ALLOW_UNSAFE_TOOLS=1 only for trusted debugging.",
                dangerous_word
            ));
        }
    }

    for pattern in DANGEROUS_COMMAND_PATTERNS {
        // pattern: 当前检查的危险命令片段。
        // 再做模式级命中，覆盖参数组合等高危片段。
        if normalized.contains(pattern) {
            return Err(format!(
                "Blocked by permission gate: command contains dangerous pattern '{}'. Set NOVA_ALLOW_UNSAFE_TOOLS=1 only for trusted debugging.",
                pattern
            ));
        }
    }

    Ok(())
}

fn check_file_path(path: &str) -> Result<(), String> {
    let normalized = normalize_path_for_match(path);
    // normalized: 规范化后用于路径风险匹配的路径字符串。
    if normalized.is_empty() {
        return Err("Blocked by permission gate: target path is empty".to_string());
    }

    for prefix in PROTECTED_PATH_PREFIXES {
        // prefix: 当前检查的受保护路径前缀。
        // 前缀命中用于阻止系统目录写入。
        if normalized.starts_with(prefix) {
            return Err(format!(
                "Blocked by permission gate: writing protected path '{}'. Set NOVA_ALLOW_UNSAFE_TOOLS=1 only for trusted debugging.",
                path
            ));
        }
    }

    for marker in PROTECTED_PATH_CONTAINS {
        // marker: 当前检查的敏感路径标记。
        // contains 命中用于阻止凭据/密钥等敏感目录。
        if normalized.contains(marker) {
            return Err(format!(
                "Blocked by permission gate: writing sensitive path '{}'. Set NOVA_ALLOW_UNSAFE_TOOLS=1 only for trusted debugging.",
                path
            ));
        }
    }

    Ok(())
}

pub fn enforce_tool_permission(
    _app: &AppHandle,
    conversation_id: Option<&str>,
    tool_name: &str,
    input: &Value,
) -> PermissionEnforcement {
    // Decision order: unsafe override > deny cache > session allow > one-time allow > ask user.
    if unsafe_override_enabled() {
        // 显式调试开关打开时直接放行，不进入任何审批状态机。
        return PermissionEnforcement::Allow;
    }

    let Some(operation) = operation_from_input(tool_name, input) else {
        // operation: None 表示该工具没有声明受控操作，当前权限层不参与拦截。
        return PermissionEnforcement::Allow;
    };
    // operation: 当前待评估的受控操作。

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

    if state.deny_session.contains(&operation.signature) {
        // 会话级拒绝优先级最高，直接阻断。
        return PermissionEnforcement::Deny(format!(
            "Blocked by permission gate: this operation was denied in current session ({})",
            operation.preview
        ));
    }

    if state.allow_session.contains(&operation.signature) {
        // 会话级允许可重复使用。
        return PermissionEnforcement::Allow;
    }

    if state.allow_once.remove(&operation.signature) {
        // 一次性允许命中后立即消费，确保只生效一次。
        return PermissionEnforcement::Allow;
    }

    if operation.needs_approval {
        let request_id = upsert_pending_request_id(state, &operation);
        // request_id: 生成或复用的待审批请求 id。
        return PermissionEnforcement::AskUser {
            request_id: request_id.clone(),
            payload: build_permission_prompt_payload(&operation),
        };
    }

    PermissionEnforcement::Allow
}
