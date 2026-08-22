//! 渐进式工具披露：会话级的延迟工具加载状态（内存缓存 + 磁盘持久化）。
//!
//! Deferred 工具默认不进入模型的工具清单；模型调用 LoadTool 加载后，
//! 该工具在当前会话持续可见。状态按会话落盘，恢复历史会话后依然有效。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager};

const DISCLOSED_TOOLS_DIR_NAME: &str = "disclosed_tools";
const GLOBAL_SCOPE: &str = "__global__";

// 进程内缓存：首次访问某会话时从磁盘懒加载，之后读写都走内存。
static STATE: Lazy<Mutex<HashMap<String, HashSet<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        // 子代理会话归一化到父会话：披露状态随父会话共享。
        .map(crate::llm::services::subagent::parent_conversation_id)
        .unwrap_or(GLOBAL_SCOPE)
        .to_string()
}

fn disclosure_file(app: &AppHandle, scope: &str) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(DISCLOSED_TOOLS_DIR_NAME).join(format!("{}.json", scope)))
        .map_err(|e| format!("Failed to resolve app_data_dir for disclosed tools: {}", e))
}

fn load_from_disk(app: &AppHandle, scope: &str) -> HashSet<String> {
    let Ok(path) = disclosure_file(app, scope) else {
        return HashSet::new();
    };
    if !path.exists() {
        return HashSet::new();
    }
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return HashSet::new();
    };
    serde_json::from_str::<Vec<String>>(&raw)
        .map(|names| names.into_iter().collect())
        .unwrap_or_else(|error| {
            tracing::warn!(error = %error, path = %path.display(), "disclosed tools parse failed");
            HashSet::new()
        })
}

fn persist_to_disk(app: &AppHandle, scope: &str, names: &HashSet<String>) {
    let Ok(path) = disclosure_file(app, scope) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    match serde_json::to_string_pretty(&sorted) {
        Ok(content) => {
            if let Err(error) = crate::llm::utils::atomic_write::write_str(&path, &content) {
                tracing::warn!(error = %error, path = %path.display(), "disclosed tools persist failed");
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "disclosed tools serialize failed");
        }
    }
}

/// 当前会话已加载的延迟工具名集合（磁盘懒加载 + 内存缓存）。
pub fn disclosed_tools(app: &AppHandle, conversation_id: Option<&str>) -> HashSet<String> {
    let scope = scope_key(conversation_id);
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(_) => return HashSet::new(),
    };
    if let Some(existing) = guard.get(&scope) {
        return existing.clone();
    }
    let loaded = load_from_disk(app, &scope);
    guard.insert(scope.clone(), loaded.clone());
    loaded
}

/// 标记工具为已加载并落盘；返回本次新加载的名字（去重后）。
pub fn mark_disclosed(
    app: &AppHandle,
    conversation_id: Option<&str>,
    names: &[String],
) -> Vec<String> {
    let scope = scope_key(conversation_id);
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(_) => return Vec::new(),
    };
    let entry = guard
        .entry(scope.clone())
        .or_insert_with(|| load_from_disk(app, &scope));

    let mut newly_loaded = Vec::new();
    for name in names {
        if entry.insert(name.clone()) {
            newly_loaded.push(name.clone());
        }
    }
    if !newly_loaded.is_empty() {
        persist_to_disk(app, &scope, entry);
    }
    newly_loaded
}

/// 移除会话披露缓存（删除会话时联动清理磁盘记录）。
pub fn forget_conversation(app: &AppHandle, conversation_id: Option<&str>) {
    let scope = scope_key(conversation_id);
    if let Ok(mut guard) = STATE.lock() {
        guard.remove(&scope);
    }
    if let Ok(path) = disclosure_file(app, &scope) {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}
