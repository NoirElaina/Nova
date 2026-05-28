mod language_server;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    language_server::registrations()
}