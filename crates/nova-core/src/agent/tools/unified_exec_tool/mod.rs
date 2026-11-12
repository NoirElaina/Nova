mod exec_command;
mod write_stdin;

use super::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![exec_command::registration(), write_stdin::registration()]
}
