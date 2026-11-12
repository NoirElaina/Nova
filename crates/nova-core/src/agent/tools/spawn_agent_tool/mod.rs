mod followup_task;
mod interrupt_agent;
mod list_agents;
mod send_message;
mod spawn_agent;
mod wait_agent;

use crate::agent::tools::ToolRegistration;

pub(crate) fn registrations() -> Vec<ToolRegistration> {
    let mut tools = Vec::with_capacity(6);
    tools.extend(spawn_agent::registrations());
    tools.extend(send_message::registrations());
    tools.extend(wait_agent::registrations());
    tools.extend(list_agents::registrations());
    tools.extend(interrupt_agent::registrations());
    tools.extend(followup_task::registrations());
    tools
}
