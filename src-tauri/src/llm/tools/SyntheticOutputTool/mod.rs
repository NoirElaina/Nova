mod synthetic_output;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![synthetic_output::registration()]
}