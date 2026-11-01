//! 持久化权限规则与审批裁决管理命令。


use crate::agent::permissions::rules::{self, PermissionRule};

pub fn list_permission_rules(core: &crate::core::Core) -> Result<Vec<PermissionRule>, String> {
    Ok(rules::load_rules(&core))
}

pub fn delete_permission_rule(core: &crate::core::Core, signature: String) -> Result<bool, String> {
    rules::remove_rule(&core, &signature)
}

pub fn submit_permission_decision(
    core: &crate::core::Core,
    conversation_id: Option<String>,
    request_id: String,
    action: String,
) -> Result<bool, String> {
    let action_enum = crate::agent::permissions::parse_permission_action_name(&action)
        .ok_or_else(|| format!("Unknown permission action: {}", action))?;
    crate::agent::permissions::submit_permission_decision(
        &core,
        conversation_id.as_deref(),
        &request_id,
        action_enum,
    )
}
