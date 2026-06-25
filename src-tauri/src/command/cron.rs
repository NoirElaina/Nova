use crate::llm::commands::types::HistoryMessage;
use crate::llm::services::cron_schedule;
use crate::llm::tools::shared::cron_store::{add_job, list_jobs, remove_job, CronJob};
use crate::llm::types::{AgentMode, Content, Message, Role};
use chrono::{Local, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tauri::{AppHandle, Emitter};
use tokio::time::{self, Duration};
use tracing::{error, warn};
use uuid::Uuid;

use crate::llm::utils::error_event::report_backend_result;

const SCHEDULER_TICK_SECONDS: u64 = 15;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskTriggerEvent {
    pub id: String,
    pub conversation_id: Option<String>,
    pub cron: String,
    pub prompt: String,
    pub recurring: bool,
    pub durable: bool,
    pub created_at: String,
    pub triggered_at: String,
}

fn build_scheduled_conversation_title(cron: &str, prompt: &str) -> String {
    let mut title_seed = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("Scheduled Task")
        .to_string();

    if title_seed.chars().count() > 48 {
        title_seed = title_seed.chars().take(48).collect::<String>();
    }

    format!("Scheduled [{}] {}", cron, title_seed)
}

async fn create_bound_conversation_for_task(
    app: &AppHandle,
    cron: &str,
    prompt: &str,
) -> Result<String, String> {
    let title = build_scheduled_conversation_title(cron, prompt);
    let conversation = crate::llm::history::create_conversation(app, Some(title), None).await?;
    Ok(conversation.id)
}

async fn append_trigger_prompt_to_bound_conversation(
    app: &AppHandle,
    job: &CronJob,
    triggered_at: &str,
) -> Result<(), String> {
    let Some(conversation_id) = job
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(());
    };

    let content = build_scheduled_trigger_user_content(job, triggered_at);

    crate::llm::history::append_history(
        app,
        conversation_id,
        HistoryMessage {
            role: "user".to_string(),
            content,
            reasoning: None,
            attachments: None,
            token_usage: None,
            cost: None,
        },
    )
    .await
}

fn build_scheduled_trigger_user_content(job: &CronJob, triggered_at: &str) -> String {
    format!(
        "[Scheduled Task Trigger]\nTask ID: {}\nCron: {}\nTriggered At: {}\n\n{}",
        job.id, job.cron, triggered_at, job.prompt
    )
}

async fn execute_scheduled_prompt_in_bound_conversation(
    app: &AppHandle,
    job: &CronJob,
    triggered_at: &str,
) -> Result<(), String> {
    let Some(conversation_id) = job
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(());
    };

    let message_content = build_scheduled_trigger_user_content(job, triggered_at);
    let turn_messages = vec![Message {
        role: Role::User,
        content: Content::Text(message_content),
    }];

    crate::llm::cancellation::begin_turn(Some(conversation_id));
    let result = crate::llm::query::send_chat_message(
        app.clone(),
        Some(conversation_id.to_string()),
        turn_messages,
        AgentMode::Agent,
    )
    .await;
    crate::llm::cancellation::finish_turn(Some(conversation_id));

    result.map_err(|e| {
        format!(
            "Failed to execute scheduled prompt for task {} in conversation {}: {}",
            job.id, conversation_id, e
        )
    })
}

pub async fn run_scheduler_loop(app: AppHandle) {
    let mut ticker = time::interval(Duration::from_secs(SCHEDULER_TICK_SECONDS));
    let mut fired_minute_by_id: HashMap<String, String> = HashMap::new();

    loop {
        ticker.tick().await;

        let now_local = Local::now();
        let now_utc = Utc::now().to_rfc3339();
        let minute_key = now_local.format("%Y-%m-%d %H:%M").to_string();

        let jobs = match list_jobs(&app) {
            Ok(v) => v,
            Err(e) => {
                error!(operation = "command.cron.run_scheduler_loop", error = %e, "failed to list scheduled jobs");
                continue;
            }
        };

        let mut existing_ids = HashSet::new();

        for job in jobs {
            existing_ids.insert(job.id.clone());

            if !cron_schedule::matches_local_minute(&job.cron, &now_local) {
                continue;
            }

            if fired_minute_by_id
                .get(&job.id)
                .map(|key| key == &minute_key)
                .unwrap_or(false)
            {
                continue;
            }

            if let Err(e) = append_trigger_prompt_to_bound_conversation(&app, &job, &now_utc).await
            {
                error!(
                    operation = "command.cron.append_trigger_prompt_to_bound_conversation",
                    job_id = %job.id,
                    error = %e,
                    "failed to append scheduled trigger prompt"
                );
            }

            let app_for_turn = app.clone();
            let job_for_turn = job.clone();
            let triggered_at_for_turn = now_utc.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = execute_scheduled_prompt_in_bound_conversation(
                    &app_for_turn,
                    &job_for_turn,
                    &triggered_at_for_turn,
                )
                .await
                {
                    error!(
                        operation = "command.cron.execute_scheduled_prompt_in_bound_conversation",
                        job_id = %job_for_turn.id,
                        error = %e,
                        "failed to execute scheduled prompt"
                    );
                }
            });

            let conversation_id = job
                .conversation_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string);

            let payload = ScheduledTaskTriggerEvent {
                id: job.id.clone(),
                conversation_id,
                cron: job.cron.clone(),
                prompt: job.prompt.clone(),
                recurring: job.recurring,
                durable: job.durable,
                created_at: job.created_at.clone(),
                triggered_at: now_utc.clone(),
            };

            match app.emit("scheduled-task-trigger", &payload) {
                Ok(_) => {
                    fired_minute_by_id.insert(job.id.clone(), minute_key.clone());
                    if !job.recurring {
                        match remove_job(&app, &job.id) {
                            Ok(true) => {
                                fired_minute_by_id.remove(&job.id);
                            }
                            Ok(false) => {
                                warn!(
                                    operation = "command.cron.run_scheduler_loop",
                                    job_id = %job.id,
                                    minute_key = %minute_key,
                                    "one-shot job was not removed after trigger; keeping minute guard"
                                );
                            }
                            Err(e) => {
                                error!(
                                    operation = "command.cron.run_scheduler_loop",
                                    job_id = %job.id,
                                    minute_key = %minute_key,
                                    error = %e,
                                    "failed to remove one-shot job after trigger"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(
                        operation = "command.cron.run_scheduler_loop",
                        job_id = %job.id,
                        error = %e,
                        "failed to emit scheduled-task-trigger event"
                    );
                }
            }
        }

        fired_minute_by_id.retain(|id, _| existing_ids.contains(id));
    }
}

#[tauri::command]
pub fn list_scheduled_tasks(app: AppHandle) -> Result<Vec<CronJob>, String> {
    report_backend_result(
        &app,
        "command.cron.list_scheduled_tasks",
        list_jobs(&app),
        None,
    )
}

#[tauri::command]
pub async fn create_scheduled_task(
    app: AppHandle,
    cron: String,
    prompt: String,
    recurring: Option<bool>,
    durable: Option<bool>,
) -> Result<CronJob, String> {
    let result = async {
        let cron_value = cron.trim();
        if cron_value.is_empty() {
            return Err("cron is required".to_string());
        }
        cron_schedule::validate_expression(cron_value)?;

        let prompt_value = prompt.trim();
        if prompt_value.is_empty() {
            return Err("prompt is required".to_string());
        }

        let raw_uuid = Uuid::new_v4().simple().to_string();
        let id = format!("cron-{}", &raw_uuid[..12]);

        let conversation_id =
            create_bound_conversation_for_task(&app, cron_value, prompt_value).await?;

        let job = CronJob {
            id,
            cron: cron_value.to_string(),
            prompt: prompt_value.to_string(),
            conversation_id: Some(conversation_id.clone()),
            recurring: recurring.unwrap_or(true),
            durable: durable.unwrap_or(false),
            created_at: Utc::now().to_rfc3339(),
        };

        match add_job(&app, job) {
            Ok(saved) => Ok(saved),
            Err(e) => {
                if let Err(cleanup_error) =
                    crate::llm::history::delete_conversation(&app, &conversation_id).await
                {
                    error!(
                        operation = "command.cron.create_scheduled_task.cleanup",
                        conversation_id = %conversation_id,
                        error = %cleanup_error,
                        "failed to cleanup conversation after add_job error"
                    );
                }
                Err(e)
            }
        }
    }
    .await;
    report_backend_result(&app, "command.cron.create_scheduled_task", result, None)
}

#[tauri::command]
pub fn delete_scheduled_task(app: AppHandle, id: String) -> Result<bool, String> {
    let result = (|| {
        let task_id = id.trim();
        if task_id.is_empty() {
            return Err("id is required".to_string());
        }

        remove_job(&app, task_id)
    })();
    report_backend_result(&app, "command.cron.delete_scheduled_task", result, None)
}
