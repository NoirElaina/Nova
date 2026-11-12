use crate::services::unified_exec;
use nova_protocol::misc::BackgroundOutput;

pub async fn list_background_jobs(
    conversation_id: Option<String>,
) -> Result<Vec<BackgroundOutput>, String> {
    Ok(unified_exec::list_processes_for_conversation(
        conversation_id.as_deref(),
    ))
}

pub async fn read_background_job_output(
    conversation_id: Option<String>,
    id: String,
    tail_lines: Option<usize>,
) -> Result<BackgroundOutput, String> {
    unified_exec::read_process_snapshot(conversation_id.as_deref(), id.trim(), tail_lines)
}

pub async fn kill_background_job(
    conversation_id: Option<String>,
    id: String,
) -> Result<BackgroundOutput, String> {
    unified_exec::kill_process(conversation_id.as_deref(), id.trim()).await
}
