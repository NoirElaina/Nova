mod reset_shell_session;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![reset_shell_session::registration()]
}