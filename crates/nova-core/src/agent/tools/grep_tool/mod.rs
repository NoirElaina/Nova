mod grep;
use super::ToolRegistration;

pub(crate) use grep::find_rg_path;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![grep::registration()]
}
