mod edit;
use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![edit::registration()]
}
