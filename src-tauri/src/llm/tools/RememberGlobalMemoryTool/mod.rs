mod remember_global_memory;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![remember_global_memory::registration()]
}