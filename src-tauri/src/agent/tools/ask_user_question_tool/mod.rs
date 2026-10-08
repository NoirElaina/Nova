mod ask_user_question;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![ask_user_question::registration()]
}