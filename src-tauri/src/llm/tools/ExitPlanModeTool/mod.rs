mod exit_plan_mode;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![exit_plan_mode::registration()]
}