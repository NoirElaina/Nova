mod list_mcp_resources;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![list_mcp_resources::registration()]
}