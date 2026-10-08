mod rag;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![rag::registration()]
}