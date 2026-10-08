mod task;
use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![task::registration()]
}
