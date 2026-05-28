mod privileged_tool;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![privileged_tool::registration()]
}