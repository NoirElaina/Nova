mod grep;
use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![grep::registration()]
}
