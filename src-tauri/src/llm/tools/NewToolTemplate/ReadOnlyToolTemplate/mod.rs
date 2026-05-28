mod read_only_tool;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![read_only_tool::registration()]
}