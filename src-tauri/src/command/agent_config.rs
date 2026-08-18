// 智能体套件命令层：bundle 增删改查 + 会话级挂载/卸载 + 前端配置所需的工具目录。
// 智能体是会话级的：set_conversation_agent 只影响指定对话，全局始终是默认 Nova。

use crate::llm::services::agent_bundles::{self, AgentBundle};
use crate::llm::utils::error_event::report_backend_result;
use serde::Serialize;
use tauri::AppHandle;

/// 前端智能体配置页展示的可配置工具（勾选清单用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurableTool {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    /// 内核流程控制工具（不可移除）。
    pub always_on: bool,
}

#[tauri::command]
pub fn list_agent_bundles(app: AppHandle) -> Result<Vec<AgentBundle>, String> {
    report_backend_result(
        &app,
        "command.agent_config.list_agent_bundles",
        agent_bundles::list_bundles(&app),
        None,
    )
}

#[tauri::command]
pub fn create_agent_bundle(app: AppHandle, name: String) -> Result<AgentBundle, String> {
    report_backend_result(
        &app,
        "command.agent_config.create_agent_bundle",
        agent_bundles::create_bundle(&app, &name),
        None,
    )
}

#[tauri::command]
pub fn load_agent_bundle(app: AppHandle, bundle_id: String) -> Result<AgentBundle, String> {
    report_backend_result(
        &app,
        "command.agent_config.load_agent_bundle",
        agent_bundles::load_bundle(&app, &bundle_id),
        None,
    )
}

#[tauri::command]
pub fn save_agent_bundle(app: AppHandle, bundle: AgentBundle) -> Result<AgentBundle, String> {
    report_backend_result(
        &app,
        "command.agent_config.save_agent_bundle",
        agent_bundles::save_bundle(&app, bundle),
        None,
    )
}

/// 删除 bundle 文件，并清空所有挂载它的会话引用（回到默认 Nova）+ 刷新缓存。
#[tauri::command]
pub async fn delete_agent_bundle(app: AppHandle, bundle_id: String) -> Result<(), String> {
    let result = async {
        agent_bundles::delete_bundle(&app, &bundle_id)?;
        crate::llm::history::clear_conversation_agent_references(&app, &bundle_id).await?;
        Ok(())
    }
    .await;
    report_backend_result(
        &app,
        "command.agent_config.delete_agent_bundle",
        result,
        None,
    )
}

/// 读取会话挂载的智能体（解析为完整 bundle；未挂载/文件已删返回 None）。
#[tauri::command]
pub async fn get_conversation_agent(
    app: AppHandle,
    conversation_id: String,
) -> Result<Option<AgentBundle>, String> {
    let result = async {
        let bundle_id = crate::llm::history::get_conversation_agent(&app, &conversation_id).await?;
        match bundle_id {
            Some(id) => Ok(agent_bundles::load_bundle(&app, &id).ok()),
            None => Ok(None),
        }
    }
    .await;
    report_backend_result(
        &app,
        "command.agent_config.get_conversation_agent",
        result,
        None,
    )
}

/// 挂载/卸载会话的智能体（bundle_id = None 卸载，回到默认 Nova）。
#[tauri::command]
pub async fn set_conversation_agent(
    app: AppHandle,
    conversation_id: String,
    bundle_id: Option<String>,
) -> Result<Option<AgentBundle>, String> {
    let result = async {
        // 挂载前确认 bundle 存在，避免会话指向已删除的文件。
        if let Some(id) = bundle_id.as_deref() {
            agent_bundles::load_bundle(&app, id)?;
        }
        crate::llm::history::set_conversation_agent(&app, &conversation_id, bundle_id.as_deref())
            .await?;
        let active_id =
            crate::llm::history::get_conversation_agent(&app, &conversation_id).await?;
        Ok(active_id.and_then(|id| agent_bundles::load_bundle(&app, &id).ok()))
    }
    .await;
    report_backend_result(
        &app,
        "command.agent_config.set_conversation_agent",
        result,
        None,
    )
}

/// 内置工具目录：智能体配置页勾选清单用。
/// 流程控制工具（计划模式/用户问答）标记 always_on，前端锁定其勾选框。
#[tauri::command]
pub fn list_configurable_tools(app: AppHandle) -> Result<Vec<ConfigurableTool>, String> {
    let result = crate::llm::tools::configurable_tool_catalog(&app);
    report_backend_result(
        &app,
        "command.agent_config.list_configurable_tools",
        result,
        None,
    )
}
