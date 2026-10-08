//! 动态工具注册中心 (Dynamic Tool Registry)
//!
//! 支持内置原生工具聚合与运行期动态工具注入（脚本、插件、扩展等）。
//! 提供 O(1) 大小写无关查找、统一的权限描述与安全分级，
//! 消除原本高频线性遍历与重复构造 JSON Schema 的内存开销。

use crate::agent::tools::{
    ToolDisclosure, ToolExecResult, ToolPermissionDescriptor, ToolRegistration,
};
use crate::provider::types::Tool;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};
use tauri::AppHandle;

pub(crate) type BoxToolFuture<'a> = Pin<Box<dyn Future<Output = ToolExecResult> + Send + 'a>>;

/// 运行时自定义/动态工具特征 (Trait)。
/// 供后续动态脚本、扩展插件、本地自建工具等统一接入。
pub(crate) trait CustomToolHandler: Send + Sync {
    /// 工具名称与 JSON Schema 定义
    fn definition(&self) -> Tool;

    /// 是否为只读工具（只读工具可参与多工具并发执行队列）
    fn is_read_only(&self) -> bool {
        false
    }

    /// 敏感操作权限描述（需要审批时返回描述符，否则返回 None）
    fn permission(&self, _input: &Value) -> Option<ToolPermissionDescriptor> {
        None
    }

    /// 渐进式披露分级
    fn disclosure(&self) -> ToolDisclosure {
        ToolDisclosure::Core
    }

    /// 执行工具
    fn execute<'a>(
        &'a self,
        app: AppHandle,
        conversation_id: Option<String>,
        input: Value,
    ) -> BoxToolFuture<'a>;
}

/// 统一工具项（包装内置工具与动态工具）
#[derive(Clone)]
pub(crate) enum ToolEntry {
    Builtin(ToolRegistration),
    Dynamic(Arc<dyn CustomToolHandler>),
}

impl ToolEntry {
    pub fn is_read_only(&self) -> bool {
        match self {
            Self::Builtin(reg) => reg.read_only,
            Self::Dynamic(handler) => handler.is_read_only(),
        }
    }

    pub fn permission(&self, input: &Value) -> Option<ToolPermissionDescriptor> {
        match self {
            Self::Builtin(reg) => reg.permission.and_then(|p| p(input)),
            Self::Dynamic(handler) => handler.permission(input),
        }
    }

    pub fn disclosure(&self) -> ToolDisclosure {
        match self {
            Self::Builtin(reg) => reg.disclosure,
            Self::Dynamic(handler) => handler.disclosure(),
        }
    }

    pub async fn execute(
        &self,
        app: AppHandle,
        conversation_id: Option<String>,
        input: Value,
    ) -> ToolExecResult {
        match self {
            Self::Builtin(reg) => (reg.execute_with_app)(app, conversation_id, input).await,
            Self::Dynamic(handler) => handler.execute(app, conversation_id, input).await,
        }
    }
}

/// 统一工具注册中心
pub(crate) struct ToolRegistry {
    /// 内置核心工具 (O(1) 静态索引，小写键名)
    builtin_tools: HashMap<String, ToolEntry>,
    /// 内置核心工具预热定义 (O(1) Schema 缓存，杜绝重复分配)
    builtin_definitions: HashMap<String, Tool>,
    /// 运行期动态添加的工具池（线程安全读写锁）
    dynamic_tools: RwLock<HashMap<String, Arc<dyn CustomToolHandler>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let builtin_list = super::builtin_tool_registrations();
        let mut builtin_tools = HashMap::with_capacity(builtin_list.len());
        let mut builtin_definitions = HashMap::with_capacity(builtin_list.len());

        for reg in builtin_list {
            let def = (reg.tool)();
            let key = def.name.to_ascii_lowercase();
            builtin_tools.insert(key.clone(), ToolEntry::Builtin(reg));
            builtin_definitions.insert(key, def);
        }

        Self {
            builtin_tools,
            builtin_definitions,
            dynamic_tools: RwLock::new(HashMap::new()),
        }
    }

    /// 注册运行期自定义动态工具（插件/第三方工具扩展点）
    #[allow(dead_code)]
    pub fn register_dynamic(&self, handler: Arc<dyn CustomToolHandler>) {
        let def = handler.definition();
        let key = def.name.to_ascii_lowercase();
        if let Ok(mut guard) = self.dynamic_tools.write() {
            guard.insert(key, handler);
        }
    }

    /// 卸载运行期自定义动态工具
    #[allow(dead_code)]
    pub fn unregister_dynamic(&self, name: &str) -> bool {
        let key = name.trim().to_ascii_lowercase();
        if let Ok(mut guard) = self.dynamic_tools.write() {
            guard.remove(&key).is_some()
        } else {
            false
        }
    }

    /// 按名称 O(1) 查找工具项
    pub(crate) fn get(&self, name: &str) -> Option<ToolEntry> {
        let key = name.trim().to_ascii_lowercase();
        if let Some(entry) = self.builtin_tools.get(&key) {
            return Some(entry.clone());
        }
        if let Ok(guard) = self.dynamic_tools.read() {
            if let Some(handler) = guard.get(&key) {
                return Some(ToolEntry::Dynamic(handler.clone()));
            }
        }
        None
    }

    /// 按名称 O(1) 查找工具定义
    pub fn get_definition(&self, name: &str) -> Option<Tool> {
        let key = name.trim().to_ascii_lowercase();
        if let Some(def) = self.builtin_definitions.get(&key) {
            return Some(def.clone());
        }
        if let Ok(guard) = self.dynamic_tools.read() {
            if let Some(handler) = guard.get(&key) {
                return Some(handler.definition());
            }
        }
        None
    }

    /// 列出所有工具的定义（内置 + 动态）
    pub fn list_all_tools(&self) -> Vec<Tool> {
        let mut list: Vec<Tool> = self.builtin_definitions.values().cloned().collect();
        if let Ok(guard) = self.dynamic_tools.read() {
            for handler in guard.values() {
                list.push(handler.definition());
            }
        }
        list
    }

    /// 查询某工具是否只读
    pub fn is_read_only(&self, name: &str) -> Option<bool> {
        self.get(name).map(|entry| entry.is_read_only())
    }

    /// 查询某工具的敏感权限描述
    pub fn permission_descriptor(&self, name: &str, input: &Value) -> Option<ToolPermissionDescriptor> {
        self.get(name).and_then(|entry| entry.permission(input))
    }

    /// 查询某工具的披露分级
    pub fn disclosure(&self, name: &str) -> Option<ToolDisclosure> {
        self.get(name).map(|entry| entry.disclosure())
    }
}

/// 全局工具注册中心单例
pub(crate) fn global_tool_registry() -> &'static ToolRegistry {
    static REGISTRY: OnceLock<ToolRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ToolRegistry::new)
}
