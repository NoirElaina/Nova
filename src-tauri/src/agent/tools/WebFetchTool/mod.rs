mod web_fetch;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![web_fetch::registration()]
}