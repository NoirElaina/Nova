mod read_session_file;
use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![read_session_file::registration()]
}
