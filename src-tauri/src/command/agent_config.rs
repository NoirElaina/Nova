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

// ---------------- 智能体目录资源：私有技能 / 资料文件 / 私有 MCP ----------------

/// 某智能体的私有技能列表（agents/<id>/skills/）。
#[tauri::command]
pub fn list_agent_private_skills(
    app: AppHandle,
    bundle_id: String,
) -> Result<Vec<crate::llm::services::skills::SkillSummary>, String> {
    report_backend_result(
        &app,
        "command.agent_config.list_agent_private_skills",
        crate::llm::services::skills::list_agent_private_skill_summaries(&app, &bundle_id),
        None,
    )
}

/// 智能体资料文件条目（agents/<id>/files/ 顶层）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileEntry {
    pub name: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

#[tauri::command]
pub fn list_agent_files(app: AppHandle, bundle_id: String) -> Result<Vec<AgentFileEntry>, String> {
    let result = (|| {
        let dir = agent_bundles::agent_files_dir(&app, &bundle_id)?;
        let mut out = Vec::new();
        if dir.exists() {
            for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let meta = entry.metadata().map_err(|e| e.to_string())?;
                let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                out.push(AgentFileEntry {
                    name,
                    size_bytes: meta.len(),
                    is_dir: meta.is_dir(),
                });
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    })();
    report_backend_result(&app, "command.agent_config.list_agent_files", result, None)
}

/// 导入资料文件到智能体目录（复制进 agents/<id>/files/，同名覆盖）。
#[tauri::command]
pub fn import_agent_file(
    app: AppHandle,
    bundle_id: String,
    src_path: String,
) -> Result<AgentFileEntry, String> {
    let result = (|| {
        let src = std::path::Path::new(&src_path);
        if !src.is_file() {
            return Err("源路径必须是文件".to_string());
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("无法解析文件名")?
            .to_string();
        // 文件名安全：不允许路径分隔符（防穿越）。
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err("文件名包含非法字符".to_string());
        }
        let dir = agent_bundles::agent_files_dir(&app, &bundle_id)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = dir.join(&name);
        std::fs::copy(src, &dest).map_err(|e| format!("复制文件失败: {}", e))?;
        Ok(AgentFileEntry {
            name,
            size_bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
            is_dir: false,
        })
    })();
    report_backend_result(&app, "command.agent_config.import_agent_file", result, None)
}

/// 删除智能体资料文件/子目录（仅限 files/ 顶层）。
#[tauri::command]
pub fn delete_agent_file(app: AppHandle, bundle_id: String, name: String) -> Result<(), String> {
    let result = (|| {
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err("文件名包含非法字符".to_string());
        }
        let dir = agent_bundles::agent_files_dir(&app, &bundle_id)?;
        let target = dir.join(&name);
        if !target.exists() {
            return Err(format!("文件不存在: {}", name));
        }
        if target.is_dir() {
            std::fs::remove_dir_all(&target).map_err(|e| e.to_string())
        } else {
            std::fs::remove_file(&target).map_err(|e| e.to_string())
        }
    })();
    report_backend_result(&app, "command.agent_config.delete_agent_file", result, None)
}

/// 在系统资源管理器中打开智能体目录（用户可直接放入 SKILL.md / 资料文件）。
#[tauri::command]
pub fn reveal_agent_dir(app: AppHandle, bundle_id: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let result = (|| {
        let dir = agent_bundles::agent_dir(&app, &bundle_id)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.to_string_lossy().to_string();
        app.opener()
            .open_path(&path, None::<&str>)
            .map_err(|e| format!("打开目录失败: {}", e))
    })();
    report_backend_result(&app, "command.agent_config.reveal_agent_dir", result, None)
}

/// 某智能体的私有 MCP server 列表（读 agents/<id>/mcp.json）。
#[tauri::command]
pub async fn list_agent_mcp_servers(
    app: AppHandle,
    bundle_id: String,
) -> Result<Vec<crate::llm::services::mcp::McpServerEntry>, String> {
    let result = async {
        crate::llm::services::mcp::agent_mcp_server_entries(&app, &bundle_id).await
    }
    .await;
    report_backend_result(
        &app,
        "command.agent_config.list_agent_mcp_servers",
        result,
        None,
    )
}

/// 智能体私有 MCP 增/改/删。
/// old_name = None 新增；new_name = None 删除；两者都有 = 修改/重命名。
#[tauri::command]
pub async fn upsert_agent_mcp_server(
    app: AppHandle,
    bundle_id: String,
    old_name: Option<String>,
    new_name: Option<String>,
    config: Option<crate::llm::services::mcp::McpServerConfig>,
    enabled: Option<bool>,
) -> Result<(), String> {
    let result = crate::llm::services::mcp::upsert_agent_mcp_server(
        &app,
        &bundle_id,
        old_name,
        new_name,
        config,
        enabled.unwrap_or(true),
    )
    .await;
    report_backend_result(
        &app,
        "command.agent_config.upsert_agent_mcp_server",
        result,
        None,
    )
}
