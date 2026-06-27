// 文件读取状态注册表——对齐 claude-code 的 readFileState 机制。
//
// 核心约束：EditTool 在写入前必须确认该文件在当前会话内已被 ReadTool 读过，
// 且读取后磁盘 mtime 未漂移。这避免 AI 凭记忆（可能已过时）盲改文件。
//
// 作用域：per-conversation_id 隔离。会话 A 读过的文件，会话 B 不能直接编辑——
// B 必须自己先 Read。这与 claude-code 的 per-session readFileState 行为一致。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// 单次 Read 留下的快照。
#[derive(Clone)]
pub struct ReadFileState {
    /// 读取时刻的文件 mtime。写前对比当前 mtime 检测漂移。
    pub mtime: SystemTime,
    /// 完整读取时缓存的内容（已 strip BOM）。用于 Windows mtime 误报 fallback——
    /// 云同步/杀毒软件可能改 mtime 但内容未变，此时字节对比通过则放行。
    /// 分页读取（offset/limit）时为 None。
    pub content: Option<String>,
    /// 是否分页读取。分页读取不允许直接编辑——AI 看到的不是完整文件。
    pub is_partial: bool,
}

pub struct ReadFileRegistry {
    // conversation_id -> (path -> state)
    // None conversation_id 归到 "__global__" 桶。
    inner: Mutex<HashMap<String, HashMap<PathBuf, ReadFileState>>>,
}

impl ReadFileRegistry {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn bucket_key(conversation_id: Option<&str>) -> String {
        conversation_id.unwrap_or("__global__").to_string()
    }

    /// ReadTool 成功读取后调用。记录文件 mtime + 内容快照。
    pub fn record_read(
        &self,
        conversation_id: Option<&str>,
        path: &Path,
        content: Option<String>,
        is_partial: bool,
    ) {
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let key = Self::bucket_key(conversation_id);
        let mut guard = self.inner.lock().unwrap();
        let session = guard.entry(key).or_default();
        session.insert(
            path.to_path_buf(),
            ReadFileState {
                mtime,
                content,
                is_partial,
            },
        );
    }

    /// EditTool/WriteTool 写入前调用。返回快照供调用方决策，或拒绝原因。
    pub fn check_editable(
        &self,
        conversation_id: Option<&str>,
        path: &Path,
    ) -> Result<ReadFileState, EditRejectReason> {
        let key = Self::bucket_key(conversation_id);
        let guard = self.inner.lock().unwrap();
        let session = guard.get(&key).ok_or(EditRejectReason::NotRead)?;
        let state = session.get(path).ok_or(EditRejectReason::NotRead)?;

        if state.is_partial {
            return Err(EditRejectReason::PartialRead);
        }

        // mtime 漂移检测
        let current_mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if current_mtime > state.mtime {
            // Windows fallback：完整读取且 content 字节一致 → 放行
            if let Some(ref recorded) = state.content {
                if let Ok(current_content) = std::fs::read_to_string(path) {
                    if current_content == *recorded {
                        return Ok(state.clone());
                    }
                }
            }
            return Err(EditRejectReason::Stale);
        }

        Ok(state.clone())
    }

    /// EditTool/WriteTool 写入后调用。更新 mtime + 新内容快照，
    /// 让后续连续编辑无需重新 Read。
    pub fn record_edit(
        &self,
        conversation_id: Option<&str>,
        path: &Path,
        new_content: String,
    ) {
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let key = Self::bucket_key(conversation_id);
        let mut guard = self.inner.lock().unwrap();
        let session = guard.entry(key).or_default();
        session.insert(
            path.to_path_buf(),
            ReadFileState {
                mtime,
                content: Some(new_content),
                is_partial: false,
            },
        );
    }

    /// 会话删除时清理对应桶，避免内存泄漏。
    pub fn clear_session(&self, conversation_id: Option<&str>) {
        let key = Self::bucket_key(conversation_id);
        let mut guard = self.inner.lock().unwrap();
        guard.remove(&key);
    }
}

#[derive(Debug)]
pub enum EditRejectReason {
    /// 当前会话从未 Read 过该文件
    NotRead,
    /// 仅分页读过，不允许直接编辑
    PartialRead,
    /// 读取后文件被外部修改（用户手改、linter、云同步等）
    Stale,
}

impl EditRejectReason {
    pub fn message(&self) -> &'static str {
        match self {
            EditRejectReason::NotRead => {
                "File has not been read yet. Read it first before writing to it."
            }
            EditRejectReason::PartialRead => {
                "File was only partially read. Read the full file before editing it."
            }
            EditRejectReason::Stale => {
                "File has been modified since read, either by the user or by a linter. \
                 Read it again before attempting to write it."
            }
        }
    }
}

/// 全局单例。所有会话共享同一注册表实例，内部按 conversation_id 隔离。
pub fn global_registry() -> &'static ReadFileRegistry {
    static REGISTRY: OnceLock<ReadFileRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ReadFileRegistry::new)
}
