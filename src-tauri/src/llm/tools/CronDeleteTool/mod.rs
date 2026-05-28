mod cron_delete;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![cron_delete::registration()]
}