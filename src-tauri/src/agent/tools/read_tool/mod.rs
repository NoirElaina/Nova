mod read;
use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![read::registration()]
}
