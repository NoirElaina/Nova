mod goal_store;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![
        goal_store::create_goal_registration(),
        goal_store::update_goal_registration(),
        goal_store::get_goal_registration(),
    ]
}
