mod read_mcp_resource;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![read_mcp_resource::registration()]
}