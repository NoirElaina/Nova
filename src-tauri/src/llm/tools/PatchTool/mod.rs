mod apply_patch;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    apply_patch::registrations()
}