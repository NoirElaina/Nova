// 文件读取状态跟踪，支撑 Edit/MultiEdit/Write 的「先读后改 + 新鲜度检测」。
//
// 语义：
//   - 模型必须先用 Read 读过文件，才能 Edit / 覆盖写；
//   - 若文件自上次读取后被外部（用户 / linter / 其他进程）改动，拒绝写入，要求重读；
//   - Read / 成功的 Edit / Write 都会刷新该状态，使同一轮内的连续编辑可继续。
//
// 新鲜度检测：mtime + content 二级：
//   - 第一级：比较 mtime，若文件 mtime > 读取时记录的 mtime，怀疑被改
//   - 第二级：mtime 变了再比 content 字符串，相同就放行
//   - 这避免了 Windows 上云同步 / 杀毒改 mtime 但内容没变时的 false positive
//
// 以「会话 + 规范化绝对路径」为 key，记录读取时刻的 mtime 与归一化 LF 内容。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Debug, Clone)]
struct ReadRecord {
    /// 读取时刻的文件 mtime（Unix epoch 秒）。
    mtime_secs: u64,
    /// 读取时刻的归一化 LF 内容（与 read_file_meta 输出一致）。
    content: String,
}

fn store() -> &'static Mutex<HashMap<String, ReadRecord>> {
    static STATE: OnceLock<Mutex<HashMap<String, ReadRecord>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

// 规范化路径作为稳定 key：尽量用 canonicalize 消除 `/` 与 `\`、`..`、大小写等差异，
// 文件不存在时（新建场景）退回 display 字符串。
fn path_key(conversation_id: Option<&str>, path: &Path) -> String {
    let canonical = std::fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string());
    format!("{}::{}", conversation_id.unwrap_or("__default__"), canonical)
}

/// 读取文件的 mtime（Unix epoch 秒）；文件不存在返回 None。
fn file_mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// 记录一次读取（或写入后的最新内容），刷新该文件的读取状态。
/// `content` 应为归一化后的 LF 文本（与 Edit 读取时一致）。
pub fn record(conversation_id: Option<&str>, path: &Path, content: &str) {
    let key = path_key(conversation_id, path);
    // 写入后立即读 mtime，确保后续 Edit 能看到最新的 mtime。
    let mtime_secs = file_mtime_secs(path).unwrap_or(0);
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    state.insert(
        key,
        ReadRecord {
            mtime_secs,
            content: content.to_string(),
        },
    );
}

/// 校验文件可被编辑/覆盖：必须读过且内容未在读取后被外部改动。
/// `current_content` 为本次操作开始时读到的归一化 LF 文本。
///
/// 二级检测：
/// 1. 比较文件当前 mtime 与读取时记录的 mtime，相同 → 放行
/// 2. mtime 变了 → 比 content 字符串，相同 → 放行（Windows 云同步/杀毒 false positive）
/// 3. content 也变了 → 拒绝
pub fn ensure_editable(
    conversation_id: Option<&str>,
    path: &Path,
    current_content: &str,
) -> Result<(), String> {
    let key = path_key(conversation_id, path);
    let state = store().lock().unwrap_or_else(|e| e.into_inner());
    let record = match state.get(&key) {
        None => {
            return Err(format!(
                "File has not been read yet: {}. Use Read to read it before editing.",
                path.display()
            ))
        }
        Some(r) => r,
    };

    // 第一级：mtime 比较
    let current_mtime = file_mtime_secs(path).unwrap_or(record.mtime_secs);
    if current_mtime == record.mtime_secs {
        return Ok(());
    }

    // 第二级：mtime 变了，比 content 字符串
    if current_content == record.content {
        return Ok(());
    }

    Err(format!(
        "File {} has been modified since it was last read (by the user, a linter, or another process). Read it again before editing.",
        path.display()
    ))
}
