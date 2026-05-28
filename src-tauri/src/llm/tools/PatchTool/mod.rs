mod file_edit;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    file_edit::registrations()
}