use chrono::{DateTime, Local, Timelike};
use croner::parser::{CronParser, Seconds, Year};
use croner::Cron;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronScheduleInfo {
    pub expression: String,
    pub human_schedule: String,
    pub next_run_at: String,
}

fn parser() -> CronParser {
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .dom_and_dow(true)
        .build()
}

pub fn parse_expression(expr: &str) -> Result<Cron, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err("cron is required".to_string());
    }

    parser()
        .parse(trimmed)
        .map_err(|e| format!("Invalid cron expression: {e}"))
}

pub fn validate_expression(expr: &str) -> Result<(), String> {
    parse_expression(expr).map(|_| ())
}

pub fn schedule_info(expr: &str, now: &DateTime<Local>) -> Result<CronScheduleInfo, String> {
    let cron = parse_expression(expr)?;
    let next = cron
        .find_next_occurrence(now, false)
        .map_err(|e| format!("Failed to calculate next cron occurrence: {e}"))?;

    Ok(CronScheduleInfo {
        expression: expr.trim().to_string(),
        human_schedule: cron.describe(),
        next_run_at: next.to_rfc3339(),
    })
}

pub fn matches_local_minute(expr: &str, now: &DateTime<Local>) -> bool {
    let Ok(cron) = parse_expression(expr) else {
        return false;
    };

    let Some(normalized) = now.with_second(0).and_then(|time| time.with_nanosecond(0)) else {
        return false;
    };

    cron.is_time_matching(&normalized).unwrap_or(false)
}
