mod config;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![config::registration()]
}