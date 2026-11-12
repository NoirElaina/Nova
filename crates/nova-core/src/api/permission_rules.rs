//! 持久化权限规则与审批裁决管理命令。

use crate::agent::permissions::rules::{self, PermissionRule};
use crate::NovaCoreError;

pub fn list_permission_rules(
    core: &crate::core::Core,
) -> Result<Vec<PermissionRule>, NovaCoreError> {
    Ok(rules::load_rules(core))
}

pub fn delete_permission_rule(
    core: &crate::core::Core,
    signature: String,
) -> Result<bool, NovaCoreError> {
    rules::remove_rule(core, &signature).map_err(NovaCoreError::Session)
}

pub fn submit_permission_decision(
    core: &crate::core::Core,
    conversation_id: Option<String>,
    request_id: String,
    action: String,
) -> Result<bool, NovaCoreError> {
    let action_enum = crate::agent::permissions::parse_permission_action_name(&action)
        .ok_or_else(|| NovaCoreError::InvalidInput(format!("Unknown permission action: {}", action)))?;
    crate::agent::permissions::submit_permission_decision(
        core,
        conversation_id.as_deref(),
        &request_id,
        action_enum,
    )
    .map_err(NovaCoreError::Internal)
}
