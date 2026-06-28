// TodoWrite 工具的 per-conversation 状态注册表。
//
// 核心用途：让 agent 在多步骤工程任务中维护一份待办清单，跟踪进度。
// 对齐 Claude Code 的 TodoWrite 行为：整列表替换，状态机 pending/in_progress/completed。
//
// 作用域：per-conversation_id 隔离。会话 A 的待办不会泄漏到会话 B。
// 会话删除时调用 clear_session 清理，避免内存泄漏。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TodoEntry {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
}

pub struct TodoRegistry {
    // conversation_id -> todos
    // None conversation_id 归到 "__global__" 桶。
    inner: Mutex<HashMap<String, Vec<TodoEntry>>>,
}

impl TodoRegistry {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn bucket_key(conversation_id: Option<&str>) -> String {
        conversation_id.unwrap_or("__global__").to_string()
    }

    /// 整列表替换。agent 每次调用 TodoWrite 都传完整列表。
    pub fn replace_all(
        &self,
        conversation_id: Option<&str>,
        todos: Vec<TodoEntry>,
    ) -> Vec<TodoEntry> {
        let key = Self::bucket_key(conversation_id);
        let mut guard = self.inner.lock().unwrap();
        guard.insert(key, todos.clone());
        todos
    }

    /// 读取当前会话的待办列表。没有则返回空。
    pub fn list(&self, conversation_id: Option<&str>) -> Vec<TodoEntry> {
        let key = Self::bucket_key(conversation_id);
        let guard = self.inner.lock().unwrap();
        guard.get(&key).cloned().unwrap_or_default()
    }

    /// 会话删除时清理对应桶。
    pub fn clear_session(&self, conversation_id: Option<&str>) {
        let key = Self::bucket_key(conversation_id);
        let mut guard = self.inner.lock().unwrap();
        guard.remove(&key);
    }
}

/// 全局单例。所有会话共享同一注册表实例，内部按 conversation_id 隔离。
pub fn global_registry() -> &'static TodoRegistry {
    static REGISTRY: OnceLock<TodoRegistry> = OnceLock::new();
    REGISTRY.get_or_init(TodoRegistry::new)
}
