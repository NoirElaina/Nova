// 文件读取状态跟踪，支撑 Edit/MultiEdit/Write 的「先读后改 + 新鲜度检测」。
//
// 语义对齐 Claude Code 的 readFileState：
//   - 模型必须先用 Read 读过文件，才能 Edit / 覆盖写；
//   - 若文件自上次读取后被外部（用户 / linter / 其他进程）改动，拒绝写入，要求重读；
//   - Read / 成功的 Edit / Write 都会刷新该状态，使同一轮内的连续编辑可继续。
//
// 以「会话 + 规范化绝对路径」为 key，记录读取时刻内容的哈希（基于归一化后的 LF 文本）。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

fn store() -> &'static Mutex<HashMap<String, u64>> {
    static STATE: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hash_content(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

// 规范化路径作为稳定 key：尽量用 canonicalize 消除 `/` 与 `\`、`..`、大小写等差异，
// 文件不存在时（新建场景）退回 display 字符串。
fn path_key(conversation_id: Option<&str>, path: &Path) -> String {
    let canonical = std::fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string());
    format!("{}::{}", conversation_id.unwrap_or("__default__"), canonical)
}

/// 记录一次读取（或写入后的最新内容），刷新该文件的读取状态。
/// `content` 应为归一化后的 LF 文本（与 Edit 读取时一致）。
pub fn record(conversation_id: Option<&str>, path: &Path, content: &str) {
    let key = path_key(conversation_id, path);
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    state.insert(key, hash_content(content));
}

/// 校验文件可被编辑/覆盖：必须读过且内容未在读取后被外部改动。
/// `current_content` 为本次操作开始时读到的归一化 LF 文本。
pub fn ensure_editable(
    conversation_id: Option<&str>,
    path: &Path,
    current_content: &str,
) -> Result<(), String> {
    let key = path_key(conversation_id, path);
    let state = store().lock().unwrap_or_else(|e| e.into_inner());
    match state.get(&key) {
        None => Err(format!(
            "File has not been read yet: {}. Use Read to read it before editing.",
            path.display()
        )),
        Some(recorded) if *recorded != hash_content(current_content) => Err(format!(
            "File {} has been modified since it was last read (by the user, a linter, or another process). Read it again before editing.",
            path.display()
        )),
        Some(_) => Ok(()),
    }
}
