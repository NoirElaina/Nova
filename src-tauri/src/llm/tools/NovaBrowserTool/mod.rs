mod click;
mod navigate;
mod reset;
mod snapshot;
mod type_text;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![
        navigate::registration(),
        snapshot::registration(),
        click::registration(),
        type_text::registration(),
        reset::registration(),
    ]
}
