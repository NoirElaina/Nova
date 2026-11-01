mod bash_kill;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![bash_kill::registration()]
}
