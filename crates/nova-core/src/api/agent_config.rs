// 智能体套件命令层：bundle 增删改查 + 会话级挂载/卸载 + 前端配置所需的工具目录。
// 智能体是会话级的：set_conversation_agent 只影响指定对话，全局始终是默认 Nova。

use crate::agent::capabilities::agent_bundles::{self, AgentBundle};
use crate::agent::utils::error_event::report_backend_result;



pub fn list_agent_bundles(core: &crate::core::Core) -> Result<Vec<AgentBundle>, String> {
    report_backend_result(
        core,
        "command.agent_config.list_agent_bundles",
        agent_bundles::list_bundles(core),
        None,
    )
}

pub fn create_agent_bundle(core: &crate::core::Core, name: String) -> Result<AgentBundle, String> {
    report_backend_result(
        core,
        "command.agent_config.create_agent_bundle",
        agent_bundles::create_bundle(core, &name),
        None,
    )
}

pub fn load_agent_bundle(core: &crate::core::Core, bundle_id: String) -> Result<AgentBundle, String> {
    report_backend_result(
        core,
        "command.agent_config.load_agent_bundle",
        agent_bundles::load_bundle(core, &bundle_id),
        None,
    )
}

pub fn save_agent_bundle(core: &crate::core::Core, bundle: AgentBundle) -> Result<AgentBundle, String> {
    report_backend_result(
        core,
        "command.agent_config.save_agent_bundle",
        agent_bundles::save_bundle(core, bundle),
        None,
    )
}

/// 删除 bundle（目录 + 会话引用清理由 service 层一并完成）。
pub async fn delete_agent_bundle(core: &crate::core::Core, bundle_id: String) -> Result<(), String> {
    let result = agent_bundles::delete_bundle(core, &bundle_id).await;
    report_backend_result(
        core,
        "command.agent_config.delete_agent_bundle",
        result,
        None,
    )
}

/// 读取会话挂载的智能体（解析为完整 bundle；未挂载/文件已删返回 None）。
pub async fn get_conversation_agent(
    core: &crate::core::Core,
    conversation_id: String,
) -> Result<Option<AgentBundle>, String> {
    let result = async {
        let bundle_id = crate::agent::session::history::get_conversation_agent(core, &conversation_id).await?;
        match bundle_id {
            Some(id) => Ok(agent_bundles::load_bundle(core, &id).ok()),
            None => Ok(None),
        }
    }
    .await;
    report_backend_result(
        core,
        "command.agent_config.get_conversation_agent",
        result,
        None,
    )
}

/// 挂载/卸载会话的智能体（bundle_id = None 卸载，回到默认 Nova）。
pub async fn set_conversation_agent(
    core: &crate::core::Core,
    conversation_id: String,
    bundle_id: Option<String>,
) -> Result<Option<AgentBundle>, String> {
    let result = async {
        // 挂载前确认 bundle 存在且已启用，避免会话指向已删除/禁用的智能体。
        if let Some(id) = bundle_id.as_deref() {
            let bundle = agent_bundles::load_bundle(core, id)?;
            if !bundle.enabled {
                return Err(format!("智能体「{}」已禁用，无法挂载", bundle.name));
            }
        }
        crate::agent::session::history::set_conversation_agent(core, &conversation_id, bundle_id.as_deref())
            .await?;
        let active_id =
            crate::agent::session::history::get_conversation_agent(core, &conversation_id).await?;
        Ok(active_id.and_then(|id| agent_bundles::load_bundle(core, &id).ok()))
    }
    .await;
    report_backend_result(
        core,
        "command.agent_config.set_conversation_agent",
        result,
        None,
    )
}


// ---------------- 智能体目录资源：私有技能 / 资料文件 ----------------

/// 某智能体的私有技能列表（agents/<id>/skills/）。
pub fn list_agent_private_skills(
    core: &crate::core::Core,
    bundle_id: String,
) -> Result<Vec<crate::agent::capabilities::skills::SkillSummary>, String> {
    report_backend_result(
        core,
        "command.agent_config.list_agent_private_skills",
        crate::agent::capabilities::skills::list_agent_private_skill_summaries(core, &bundle_id),
        None,
    )
}

pub use nova_protocol::misc::AgentFileEntry;

pub fn list_agent_files(core: &crate::core::Core, bundle_id: String) -> Result<Vec<AgentFileEntry>, String> {
    let result = (|| {
        let dir = agent_bundles::agent_files_dir(core, &bundle_id)?;
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
    report_backend_result(core, "command.agent_config.list_agent_files", result, None)
}

/// 导入资料文件到智能体目录（复制进 agents/<id>/files/，同名覆盖）。
pub fn import_agent_file(
    core: &crate::core::Core,
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
        let dir = agent_bundles::agent_files_dir(core, &bundle_id)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = dir.join(&name);
        std::fs::copy(src, &dest).map_err(|e| format!("复制文件失败: {}", e))?;
        Ok(AgentFileEntry {
            name,
            size_bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
            is_dir: false,
        })
    })();
    report_backend_result(core, "command.agent_config.import_agent_file", result, None)
}

/// 删除智能体资料文件/子目录（仅限 files/ 顶层）。
pub fn delete_agent_file(core: &crate::core::Core, bundle_id: String, name: String) -> Result<(), String> {
    let result = (|| {
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err("文件名包含非法字符".to_string());
        }
        let dir = agent_bundles::agent_files_dir(core, &bundle_id)?;
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
    report_backend_result(core, "command.agent_config.delete_agent_file", result, None)
}

/// 在系统资源管理器中打开智能体目录（用户可直接放入 SKILL.md / 资料文件）。
/// 内核实现：跨平台直接调系统打开器，无头环境同样可用。
pub fn reveal_agent_dir(core: &crate::core::Core, bundle_id: String) -> Result<(), String> {
    let result = (|| {
        let dir = agent_bundles::agent_dir(core, &bundle_id)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        reveal_in_file_manager(&dir)
    })();
    report_backend_result(core, "command.agent_config.reveal_agent_dir", result, None)
}

#[cfg(target_os = "windows")]
fn reveal_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer").arg(dir.as_os_str()).spawn()
        .map(|_| ()).map_err(|e| format!("打开目录失败: {}", e))
}

#[cfg(not(target_os = "windows"))]
fn reveal_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("xdg-open").arg(dir.as_os_str()).spawn()
        .map(|_| ()).map_err(|e| format!("打开目录失败: {}", e))
}
