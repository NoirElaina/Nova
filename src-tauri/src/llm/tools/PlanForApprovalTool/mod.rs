mod plan_for_approval;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![plan_for_approval::registration()]
}