mod mcp_auth;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![mcp_auth::registration()]
}