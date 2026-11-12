//! 智能体拓扑图谱存储。
//!
//! 负责记录和持久化智能体之间的父子衍生关系（Parent -> Child）、
//! 运行生命周期（Open / Closed）、角色定义与任务交付物，并支持 BFS 广度优先遍历。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use nova_protocol::sync::RwLockExt;
use serde::{Deserialize, Serialize};

/// 智能体衍生边的生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadSpawnEdgeStatus {
    /// 智能体正在运行或可恢复执行。
    Open,
    /// 智能体已完成任务或已被正常关闭。
    Closed,
}

/// 智能体节点数据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub nickname: String,
    pub task: String,
    pub status: ThreadSpawnEdgeStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub result_preview: Option<String>,
}

/// 智能体拓扑图谱存储接口与内存+会话级状态实现。
#[derive(Clone, Default)]
pub struct AgentGraphStore {
    // 按 root_conversation_id 隔离图谱节点
    nodes: Arc<RwLock<HashMap<String, HashMap<String, AgentNode>>>>,
}

impl AgentGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 插入或更新智能体节点。
    pub fn upsert_node(
        &self,
        root_conversation_id: &str,
        node: AgentNode,
    ) {
        let mut guard = self.nodes.write_unpoisoned();
        let map = guard
            .entry(root_conversation_id.to_string())
            .or_default();
        map.insert(node.id.clone(), node);
    }

    /// 更新节点状态与交付物预览。
    pub fn set_node_status(
        &self,
        root_conversation_id: &str,
        node_id: &str,
        status: ThreadSpawnEdgeStatus,
        result_preview: Option<String>,
    ) -> Option<AgentNode> {
        let mut guard = self.nodes.write_unpoisoned();
        let map = guard.get_mut(root_conversation_id)?;
        let node = map.get_mut(node_id)?;
        node.status = status;
        node.updated_at = chrono::Utc::now().timestamp_millis();
        if result_preview.is_some() {
            node.result_preview = result_preview;
        }
        Some(node.clone())
    }

    /// 获取特定节点。
    pub fn get_node(
        &self,
        root_conversation_id: &str,
        node_id: &str,
    ) -> Option<AgentNode> {
        let guard = self.nodes.read_unpoisoned();
        guard
            .get(root_conversation_id)
            .and_then(|m| m.get(node_id).cloned())
    }

    /// 获取特定父节点的直接子节点列表。
    pub fn list_children(
        &self,
        root_conversation_id: &str,
        parent_id: Option<&str>,
        status_filter: Option<ThreadSpawnEdgeStatus>,
    ) -> Vec<AgentNode> {
        let guard = self.nodes.read_unpoisoned();
        let Some(map) = guard.get(root_conversation_id) else {
            return Vec::new();
        };

        let mut children: Vec<AgentNode> = map
            .values()
            .filter(|n| {
                let parent_matches = match (parent_id, &n.parent_id) {
                    (Some(p), Some(np)) => p == np,
                    (None, None) => true,
                    _ => false,
                };
                if !parent_matches {
                    return false;
                }
                if let Some(expected_status) = status_filter {
                    if n.status != expected_status {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        children.sort_by_key(|n| n.created_at);
        children
    }

    /// 广度优先遍历（BFS）获取指定节点下的全部后代智能体。
    pub fn list_descendants_bfs(
        &self,
        root_conversation_id: &str,
        start_node_id: &str,
        status_filter: Option<ThreadSpawnEdgeStatus>,
    ) -> Vec<AgentNode> {
        let guard = self.nodes.read_unpoisoned();
        let Some(map) = guard.get(root_conversation_id) else {
            return Vec::new();
        };

        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(start_node_id.to_string());

        while let Some(current_id) = queue.pop_front() {
            let mut direct_children: Vec<&AgentNode> = map
                .values()
                .filter(|n| n.parent_id.as_deref() == Some(&current_id))
                .collect();
            direct_children.sort_by_key(|n| n.created_at);

            for child in direct_children {
                let status_ok = match status_filter {
                    Some(s) => child.status == s,
                    None => true,
                };
                if status_ok {
                    result.push(child.clone());
                    queue.push_back(child.id.clone());
                }
            }
        }

        result
    }

    /// 获取整个会话图谱的所有节点列表。
    /// 会话删除时清空它的智能体图谱。
    pub fn remove_conversation(&self, root_conversation_id: &str) {
        if let Ok(mut guard) = self.nodes.write() {
            guard.remove(root_conversation_id);
        }
    }

    /// 全量清理时清空全部会话图谱。
    pub fn remove_all(&self) {
        if let Ok(mut guard) = self.nodes.write() {
            guard.clear();
        }
    }

    pub fn get_all_nodes(&self, root_conversation_id: &str) -> Vec<AgentNode> {
        let guard = self.nodes.read_unpoisoned();
        let Some(map) = guard.get(root_conversation_id) else {
            return Vec::new();
        };
        let mut list: Vec<AgentNode> = map.values().cloned().collect();
        list.sort_by_key(|n| n.created_at);
        list
    }
}

/// 全局共享图谱单例。
pub fn global_graph_store() -> &'static AgentGraphStore {
    static STORE: std::sync::OnceLock<AgentGraphStore> = std::sync::OnceLock::new();
    STORE.get_or_init(AgentGraphStore::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_upsert_and_bfs_traversal() {
        let store = AgentGraphStore::new();
        let root_id = "conv_123";

        let root_agent = AgentNode {
            id: "root_agent".to_string(),
            parent_id: None,
            role: "lead".to_string(),
            nickname: "LeadAgent".to_string(),
            task: "Coordinate overall task".to_string(),
            status: ThreadSpawnEdgeStatus::Open,
            created_at: 1000,
            updated_at: 1000,
            result_preview: None,
        };

        let child_1 = AgentNode {
            id: "child_1".to_string(),
            parent_id: Some("root_agent".to_string()),
            role: "researcher".to_string(),
            nickname: "CodeExplorer".to_string(),
            task: "Explore code architecture".to_string(),
            status: ThreadSpawnEdgeStatus::Open,
            created_at: 1010,
            updated_at: 1010,
            result_preview: None,
        };

        let child_2 = AgentNode {
            id: "child_2".to_string(),
            parent_id: Some("root_agent".to_string()),
            role: "reviewer".to_string(),
            nickname: "SafetyReviewer".to_string(),
            task: "Audit security boundaries".to_string(),
            status: ThreadSpawnEdgeStatus::Open,
            created_at: 1020,
            updated_at: 1020,
            result_preview: None,
        };

        let grandchild = AgentNode {
            id: "grandchild_1".to_string(),
            parent_id: Some("child_1".to_string()),
            role: "sub-scanner".to_string(),
            nickname: "AstScanner".to_string(),
            task: "Scan AST dependencies".to_string(),
            status: ThreadSpawnEdgeStatus::Open,
            created_at: 1030,
            updated_at: 1030,
            result_preview: None,
        };

        store.upsert_node(root_id, root_agent);
        store.upsert_node(root_id, child_1);
        store.upsert_node(root_id, child_2);
        store.upsert_node(root_id, grandchild);

        let direct_children = store.list_children(root_id, Some("root_agent"), None);
        assert_eq!(direct_children.len(), 2);
        assert_eq!(direct_children[0].id, "child_1");
        assert_eq!(direct_children[1].id, "child_2");

        let descendants = store.list_descendants_bfs(root_id, "root_agent", None);
        assert_eq!(descendants.len(), 3);
        assert_eq!(descendants[0].id, "child_1");
        assert_eq!(descendants[1].id, "child_2");
        assert_eq!(descendants[2].id, "grandchild_1");

        // 更新状态
        let updated = store.set_node_status(
            root_id,
            "child_1",
            ThreadSpawnEdgeStatus::Closed,
            Some("Research completed".to_string()),
        );
        assert!(updated.is_some());
        assert_eq!(updated.unwrap().status, ThreadSpawnEdgeStatus::Closed);

        let open_children = store.list_children(
            root_id,
            Some("root_agent"),
            Some(ThreadSpawnEdgeStatus::Open),
        );
        assert_eq!(open_children.len(), 1);
        assert_eq!(open_children[0].id, "child_2");
    }
}
