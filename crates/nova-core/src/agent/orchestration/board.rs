//! 多智能体协同看板系统。
//!
//! 提供结构化的频道（Channels）、主题讨论（Threads）、消息帖子（Posts）与异步通知。
//! 充当智能体之间的共享黑板，实现主智能体与多个子智能体之间的协同发现与成果沉淀。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};

/// 协同看板频道元数据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardChannel {
    pub name: String,
    pub description: String,
    pub created_at: i64,
    pub created_by: String,
    pub message_count: usize,
}

/// 协同看板帖子内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardPost {
    pub id: String,
    pub channel: String,
    pub thread_id: Option<String>,
    pub author_id: String,
    pub author_nickname: String,
    pub author_role: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct ConversationBoard {
    channels: HashMap<String, BoardChannel>,
    posts: Vec<BoardPost>,
}

impl ConversationBoard {
    fn new_with_defaults() -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        let mut channels = HashMap::new();

        channels.insert(
            "general".to_string(),
            BoardChannel {
                name: "general".to_string(),
                description: "综合协作与任务派发讨论".to_string(),
                created_at: now,
                created_by: "system".to_string(),
                message_count: 0,
            },
        );
        channels.insert(
            "research".to_string(),
            BoardChannel {
                name: "research".to_string(),
                description: "架构调研、代码探测与技术分析结果".to_string(),
                created_at: now,
                created_by: "system".to_string(),
                message_count: 0,
            },
        );
        channels.insert(
            "evidence".to_string(),
            BoardChannel {
                name: "evidence".to_string(),
                description: "执行证据、审查记录与交付结论".to_string(),
                created_at: now,
                created_by: "system".to_string(),
                message_count: 0,
            },
        );

        Self {
            channels,
            posts: Vec::new(),
        }
    }
}

/// 协同看板存储与调度引擎。
#[derive(Clone, Default)]
pub struct AgentMessageBoard {
    boards: Arc<RwLock<HashMap<String, ConversationBoard>>>,
    storage_override: Option<std::path::PathBuf>,
}

/// 看板落盘路径。`init_storage` 在 `Core::new` 里被调用一次。
static BOARD_STORAGE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
/// 串行化落盘写操作，避免在释放 `boards` 写锁后并发写盘出现乱序覆盖。
static PERSIST_IO_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// 单个会话保留的帖子上限，防止 JSON 文件无限膨胀。
const MAX_PERSISTED_POSTS: usize = 500;

fn load_boards_from_path(path: &std::path::Path) -> HashMap<String, ConversationBoard> {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<HashMap<String, ConversationBoard>>(&raw) {
            Ok(mut loaded) => {
                for board in loaded.values_mut() {
                    if board.posts.len() > MAX_PERSISTED_POSTS {
                        let drop = board.posts.len() - MAX_PERSISTED_POSTS;
                        board.posts.drain(..drop);
                    }
                }
                loaded
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "Failed to parse persisted collaboration board JSON; starting with empty boards"
                );
                HashMap::new()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "Failed to read collaboration board storage file"
            );
            HashMap::new()
        }
    }
}

/// 绑定数据目录并把已有看板读回内存。
///
/// 之前看板纯内存：内核一重启（或桌面端崩溃重启），所有频道与帖子全部消失。
pub fn init_storage(data_dir: &std::path::Path) {
    let path = data_dir.join("collaboration_board.json");
    let loaded = load_boards_from_path(&path);
    if !loaded.is_empty() {
        if let Ok(mut guard) = global_message_board().boards.write() {
            *guard = loaded;
        }
    }
    let _ = BOARD_STORAGE.set(path);
}

fn persist_snapshot_atomic(
    path: &std::path::Path,
    boards: &RwLock<HashMap<String, ConversationBoard>>,
) {
    let _io_guard = PERSIST_IO_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut snapshot: HashMap<String, ConversationBoard> = {
        let guard = boards.read().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    };
    for board in snapshot.values_mut() {
        if board.posts.len() > MAX_PERSISTED_POSTS {
            let drop = board.posts.len() - MAX_PERSISTED_POSTS;
            board.posts.drain(..drop);
        }
    }
    let Ok(text) = serde_json::to_string(&snapshot) else {
        tracing::warn!("Failed to serialize collaboration board snapshot");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), error = %err, "Failed to create board dir");
            return;
        }
    }
    // 原子临时文件 + rename 替换，避免写一半崩溃破坏 JSON。
    let tmp_name = format!(
        ".{}.tmp.{}.{:?}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("collaboration_board.json"),
        std::process::id(),
        std::thread::current().id()
    );
    let tmp_path = path.with_file_name(tmp_name);
    if let Err(err) = std::fs::write(&tmp_path, text) {
        tracing::warn!(path = %tmp_path.display(), error = %err, "Failed to write temp board file");
        let _ = std::fs::remove_file(&tmp_path);
        return;
    }
    if let Err(err) = std::fs::rename(&tmp_path, path) {
        tracing::warn!(
            from = %tmp_path.display(),
            to = %path.display(),
            error = %err,
            "Failed to atomically rename temp collaboration board file"
        );
        let _ = std::fs::remove_file(&tmp_path);
    }
}

impl AgentMessageBoard {
    pub fn new() -> Self {
        Self {
            boards: Arc::new(RwLock::new(HashMap::new())),
            storage_override: None,
        }
    }

    #[cfg(test)]
    fn with_storage_file(path: std::path::PathBuf) -> Self {
        let loaded = load_boards_from_path(&path);
        Self {
            boards: Arc::new(RwLock::new(loaded)),
            storage_override: Some(path),
        }
    }

    fn storage_path(&self) -> Option<&std::path::Path> {
        self.storage_override
            .as_deref()
            .or_else(|| BOARD_STORAGE.get().map(|p| p.as_path()))
    }

    fn persist_async_safe(&self) {
        if let Some(path) = self.storage_path() {
            persist_snapshot_atomic(path, &self.boards);
        }
    }

    /// 会话删除时清掉它的看板，避免进程内 Map 与落盘文件无限增长。
    pub fn remove_conversation(&self, root_conversation_id: &str) {
        let removed = {
            let mut guard = self.boards.write().unwrap_or_else(|e| e.into_inner());
            guard.remove(root_conversation_id).is_some()
        };
        if removed {
            self.persist_async_safe();
        }
    }

    /// 全量清理时移除全部会话看板并刷新持久化文件。
    pub fn remove_all(&self) {
        let had_entries = {
            let mut guard = self.boards.write().unwrap_or_else(|e| e.into_inner());
            let empty = guard.is_empty();
            guard.clear();
            !empty
        };
        if had_entries {
            self.persist_async_safe();
        }
    }

    fn with_board_mut<F, R>(&self, root_conversation_id: &str, f: F) -> R
    where
        F: FnOnce(&mut ConversationBoard) -> R,
    {
        let result = {
            let mut guard = self.boards.write().unwrap_or_else(|e| e.into_inner());
            let board = guard
                .entry(root_conversation_id.to_string())
                .or_insert_with(ConversationBoard::new_with_defaults);
            let res = f(board);
            if board.posts.len() > MAX_PERSISTED_POSTS {
                let drop = board.posts.len() - MAX_PERSISTED_POSTS;
                board.posts.drain(..drop);
            }
            res
        };
        self.persist_async_safe();
        result
    }

    fn with_board_read<F, R>(&self, root_conversation_id: &str, f: F) -> R
    where
        F: FnOnce(&ConversationBoard) -> R,
    {
        let guard = self.boards.read().unwrap_or_else(|e| e.into_inner());
        if let Some(board) = guard.get(root_conversation_id) {
            return f(board);
        }
        drop(guard);
        let default_board = ConversationBoard::new_with_defaults();
        f(&default_board)
    }

    /// 创建自定义频道。
    pub fn create_channel(
        &self,
        root_conversation_id: &str,
        name: &str,
        description: &str,
        creator: &str,
    ) -> Result<BoardChannel, String> {
        let clean_name = name.trim().trim_start_matches('#').to_lowercase();
        if clean_name.is_empty() {
            return Err("Channel name cannot be empty".to_string());
        }

        self.with_board_mut(root_conversation_id, |board| {
            if let Some(existing) = board.channels.get(&clean_name) {
                return Ok(existing.clone());
            }

            let channel = BoardChannel {
                name: clean_name.clone(),
                description: description.trim().to_string(),
                created_at: chrono::Utc::now().timestamp_millis(),
                created_by: creator.to_string(),
                message_count: 0,
            };

            board.channels.insert(clean_name, channel.clone());
            Ok(channel)
        })
    }

    /// 获取所有频道列表。
    pub fn list_channels(&self, root_conversation_id: &str) -> Vec<BoardChannel> {
        self.with_board_read(root_conversation_id, |board| {
            let mut list: Vec<BoardChannel> = board.channels.values().cloned().collect();
            list.sort_by_key(|c| c.created_at);
            list
        })
    }

    /// 发布新帖子或回复主题。
    // M2 接入 idempotencyKey 时收口为结构化参数对象
    #[allow(clippy::too_many_arguments)]
    pub fn post_message(
        &self,
        root_conversation_id: &str,
        channel: &str,
        thread_id: Option<String>,
        author_id: &str,
        author_nickname: &str,
        author_role: &str,
        content: &str,
    ) -> Result<BoardPost, String> {
        let clean_channel = channel.trim().trim_start_matches('#').to_lowercase();
        if content.trim().is_empty() {
            return Err("Post content cannot be empty".to_string());
        }

        self.with_board_mut(root_conversation_id, |board| {
            if !board.channels.contains_key(&clean_channel) {
                board.channels.insert(
                    clean_channel.clone(),
                    BoardChannel {
                        name: clean_channel.clone(),
                        description: format!("动态创建频道 #{clean_channel}"),
                        created_at: chrono::Utc::now().timestamp_millis(),
                        created_by: author_nickname.to_string(),
                        message_count: 0,
                    },
                );
            }

            if let Some(chan) = board.channels.get_mut(&clean_channel) {
                chan.message_count += 1;
            }

            let post = BoardPost {
                id: uuid::Uuid::new_v4().to_string(),
                channel: clean_channel,
                thread_id,
                author_id: author_id.to_string(),
                author_nickname: author_nickname.to_string(),
                author_role: author_role.to_string(),
                content: content.trim().to_string(),
                created_at: chrono::Utc::now().timestamp_millis(),
            };

            board.posts.push(post.clone());
            if board.posts.len() > MAX_PERSISTED_POSTS {
                let overflow = board.posts.len() - MAX_PERSISTED_POSTS;
                board.posts.drain(0..overflow);
            }
            Ok(post)
        })
    }

    /// 按频道和主题筛选帖子列表。
    pub fn list_posts(
        &self,
        root_conversation_id: &str,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: usize,
    ) -> Vec<BoardPost> {
        self.with_board_read(root_conversation_id, |board| {
            let normalized_channel = channel.map(|c| c.trim().trim_start_matches('#').to_lowercase());
            board
                .posts
                .iter()
                .filter(|p| {
                    if let Some(ref target_chan) = normalized_channel {
                        if &p.channel != target_chan {
                            return false;
                        }
                    }
                    if let Some(target_thread) = thread_id {
                        if p.thread_id.as_deref() != Some(target_thread) {
                            return false;
                        }
                    }
                    true
                })
                .rev()
                .take(limit.clamp(1, 200))
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        })
    }

    /// 全文搜索帖子内容。
    pub fn search_posts(
        &self,
        root_conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<BoardPost> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        self.with_board_read(root_conversation_id, |board| {
            board
                .posts
                .iter()
                .filter(|p| {
                    p.content.to_lowercase().contains(&q)
                        || p.author_nickname.to_lowercase().contains(&q)
                        || p.channel.to_lowercase().contains(&q)
                })
                .rev()
                .take(limit.clamp(1, 100))
                .cloned()
                .collect()
        })
    }
}

/// 全局共享看板单例。
pub fn global_message_board() -> &'static AgentMessageBoard {
    static BOARD: std::sync::OnceLock<AgentMessageBoard> = std::sync::OnceLock::new();
    BOARD.get_or_init(AgentMessageBoard::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_board_default_channels_and_posting() {
        let board = AgentMessageBoard::new();
        let conv_id = "conv_test_1";

        let channels = board.list_channels(conv_id);
        assert_eq!(channels.len(), 3);
        assert!(channels.iter().any(|c| c.name == "general"));
        assert!(channels.iter().any(|c| c.name == "research"));
        assert!(channels.iter().any(|c| c.name == "evidence"));

        // 发布帖子
        let post = board
            .post_message(
                conv_id,
                "research",
                None,
                "agent_1",
                "CodeExplorer",
                "researcher",
                "Found AST parser in parser.rs",
            )
            .expect("post should succeed");

        assert_eq!(post.channel, "research");
        assert_eq!(post.author_nickname, "CodeExplorer");

        // 查询帖子
        let posts = board.list_posts(conv_id, Some("research"), None, 10);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].content, "Found AST parser in parser.rs");

        // 搜索帖子
        let search_res = board.search_posts(conv_id, "AST parser", 5);
        assert_eq!(search_res.len(), 1);
        assert_eq!(search_res[0].id, post.id);
    }

    #[test]
    fn read_unknown_conversation_does_not_leak_entry_and_persistence_is_atomic() {
        let dir = std::env::temp_dir().join(format!(
            "nova-board-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("collaboration_board.json");

        // 1. 损坏 JSON 恢复为空白而非 panic
        std::fs::write(&file, "{broken json").unwrap();
        let board = AgentMessageBoard::with_storage_file(file.clone());
        assert!(board.boards.read().unwrap().is_empty());

        // 2. 读未知会话绝不向 Map 插入状态
        assert_eq!(board.list_channels("ghost_conv").len(), 3);
        assert!(board.list_posts("ghost_conv", None, None, 10).is_empty());
        assert!(board.boards.read().unwrap().is_empty());

        // 3. 写入并原子落盘，重新加载仍保留；remove_all 清空内存与磁盘
        board
            .post_message("c1", "general", None, "a1", "Nick", "role", "persisted msg")
            .unwrap();
        let reloaded = AgentMessageBoard::with_storage_file(file.clone());
        assert_eq!(reloaded.list_posts("c1", None, None, 10).len(), 1);

        reloaded.remove_all();
        let after_clear = AgentMessageBoard::with_storage_file(file.clone());
        assert!(after_clear.boards.read().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_memory_posts_bounded_by_max_persisted_posts() {
        let board = AgentMessageBoard::new();
        let conv_id = "conv_bounded";
        let total = MAX_PERSISTED_POSTS + 25;

        for i in 0..total {
            board
                .post_message(
                    conv_id,
                    "general",
                    None,
                    "agent_1",
                    "Worker",
                    "coder",
                    &format!("post-{i}"),
                )
                .expect("post should succeed");
        }

        let guard = board.boards.read().unwrap();
        let conv_board = guard.get(conv_id).expect("board should exist");
        assert_eq!(conv_board.posts.len(), MAX_PERSISTED_POSTS);
        assert_eq!(conv_board.posts[0].content, "post-25");
        assert_eq!(
            conv_board.posts[MAX_PERSISTED_POSTS - 1].content,
            format!("post-{}", total - 1)
        );
    }
}
