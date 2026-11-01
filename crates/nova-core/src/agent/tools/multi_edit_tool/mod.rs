mod multi_edit;
use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![multi_edit::registration()]
}
