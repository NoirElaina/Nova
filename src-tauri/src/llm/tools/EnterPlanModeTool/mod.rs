mod enter_plan_mode;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![enter_plan_mode::registration()]
}