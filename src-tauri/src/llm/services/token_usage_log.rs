//! 全局 token 用量日志
//!
//! 独立于会话生命周期：会话删除时日志保留，用于跨会话聚合统计。
//! 数据源：每次 LLM 响应完成后调用 `log_token_usage`。

use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tauri::AppHandle;

use crate::llm::history::get_pool_with_schema;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageRecord {
    pub id: i64,
    pub conversation_id: Option<String>,
    pub model: String,
    pub provider: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<String>,
    pub source: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub total_sessions: i64,
    pub total_messages: i64,
    pub total_tokens: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: String,
    pub active_days: i64,
    pub current_streak: i64,
    pub peak_hour: Option<i64>,
    pub favorite_model: Option<String>,
    pub heatmap: Vec<HeatmapPoint>,
    pub model_breakdown: Vec<ModelBreakdown>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapPoint {
    pub date: String,
    pub tokens: i64,
    pub sessions: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBreakdown {
    pub model: String,
    pub tokens: i64,
    pub calls: i64,
    pub cost_usd: String,
}

pub async fn log_token_usage(
    app: &AppHandle,
    conversation_id: Option<&str>,
    model: &str,
    provider: Option<&str>,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    cost_usd: Option<&str>,
    source: Option<&str>,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let now = chrono::Utc::now().timestamp();
    let total_tokens = input_tokens + output_tokens;

    sqlx::query(
        r#"
        INSERT INTO token_usage_log (
            conversation_id, model, provider,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            total_tokens, cost_usd, source, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(conversation_id)
    .bind(model)
    .bind(provider)
    .bind(input_tokens as i64)
    .bind(output_tokens as i64)
    .bind(cache_read_tokens as i64)
    .bind(cache_creation_tokens as i64)
    .bind(total_tokens as i64)
    .bind(cost_usd)
    .bind(source)
    .bind(now)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub async fn list_token_usage(
    app: &AppHandle,
    limit: Option<i64>,
) -> Result<Vec<TokenUsageRecord>, String> {
    let pool = get_pool_with_schema(app).await?;
    let limit = limit.unwrap_or(100).clamp(1, 1000);

    let rows = sqlx::query(
        r#"
        SELECT id, conversation_id, model, provider,
               input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
               total_tokens, cost_usd, source, created_at
        FROM token_usage_log
        ORDER BY created_at DESC, id DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| TokenUsageRecord {
            id: row.get("id"),
            conversation_id: row.get("conversation_id"),
            model: row.get("model"),
            provider: row.get("provider"),
            input_tokens: row.get("input_tokens"),
            output_tokens: row.get("output_tokens"),
            cache_read_tokens: row.get("cache_read_tokens"),
            cache_creation_tokens: row.get("cache_creation_tokens"),
            total_tokens: row.get("total_tokens"),
            cost_usd: row.get("cost_usd"),
            source: row.get("source"),
            created_at: row.get("created_at"),
        })
        .collect())
}

pub async fn get_usage_stats(app: &AppHandle) -> Result<UsageStats, String> {
    let pool = get_pool_with_schema(app).await?;

    let total_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id != ''")
            .fetch_one(&pool)
            .await
            .map_err(|e| e.to_string())?;

    let total_messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
            .fetch_one(&pool)
            .await
            .map_err(|e| e.to_string())?;

    let totals = sqlx::query(
        r#"
        SELECT
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(input_tokens), 0) AS total_input_tokens,
            COALESCE(SUM(output_tokens), 0) AS total_output_tokens
        FROM token_usage_log
        "#,
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let total_tokens: i64 = totals.get("total_tokens");
    let total_input_tokens: i64 = totals.get("total_input_tokens");
    let total_output_tokens: i64 = totals.get("total_output_tokens");

    let total_cost_usd = sum_cost(&pool).await?;

    let active_days: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT date(created_at, 'unixepoch')) FROM token_usage_log",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let current_streak = compute_current_streak(&pool).await?;

    let peak_hour: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT hour FROM (
            SELECT strftime('%H', created_at, 'unixepoch') AS hour, COUNT(*) AS cnt
            FROM token_usage_log
            GROUP BY hour
            ORDER BY cnt DESC, hour ASC
            LIMIT 1
        ) AS t
        "#,
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .and_then(|h: String| h.parse::<i64>().ok());

    let favorite_model: Option<String> = sqlx::query_scalar(
        r#"
        SELECT model FROM (
            SELECT model, SUM(total_tokens) AS tokens
            FROM token_usage_log
            GROUP BY model
            ORDER BY tokens DESC, model ASC
            LIMIT 1
        ) AS t
        "#,
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let heatmap = fetch_heatmap(&pool).await?;
    let model_breakdown = fetch_model_breakdown(&pool).await?;

    Ok(UsageStats {
        total_sessions,
        total_messages,
        total_tokens,
        total_input_tokens,
        total_output_tokens,
        total_cost_usd,
        active_days,
        current_streak,
        peak_hour,
        favorite_model,
        heatmap,
        model_breakdown,
    })
}

async fn sum_cost(pool: &SqlitePool) -> Result<String, String> {
    sum_cost_rows(pool, "SELECT cost_usd FROM token_usage_log WHERE cost_usd IS NOT NULL AND cost_usd != ''", &[])
        .await
}

fn parse_decimal(raw: &str) -> Option<(u128, u32)> {
    let value = raw.trim();
    if value.is_empty() || value.starts_with('-') {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole_units = if whole.is_empty() {
        0
    } else {
        whole.parse::<u128>().ok()?
    };
    let fraction_units = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u128>().ok()?
    };
    let scale = u32::try_from(fraction.len()).ok()?;
    let units = whole_units
        .checked_mul(pow10(scale)?)?
        .checked_add(fraction_units)?;
    Some((units, scale))
}

fn pow10(power: u32) -> Option<u128> {
    let mut value = 1u128;
    for _ in 0..power {
        value = value.checked_mul(10)?;
    }
    Some(value)
}

fn format_scaled(units: u128, scale: u32) -> String {
    if scale == 0 {
        return units.to_string();
    }
    let divisor = pow10(scale).unwrap_or(1);
    let whole = units / divisor;
    let fraction = units % divisor;
    if fraction == 0 {
        return whole.to_string();
    }
    let mut fraction_text = format!("{:0width$}", fraction, width = scale as usize);
    while fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    format!("{whole}.{fraction_text}")
}

async fn sum_cost_rows(
    pool: &SqlitePool,
    sql: &str,
    binds: &[&str],
) -> Result<String, String> {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(b);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

    let mut total_units: u128 = 0;
    const SCALE: u32 = 18;
    for row in rows {
        let value: String = row.get("cost_usd");
        if let Some((units, scale)) = parse_decimal(&value) {
            let normalized = match pow10(scale.abs_diff(SCALE)) {
                None => None,
                Some(p) => {
                    if scale > SCALE {
                        Some(units / p)
                    } else {
                        units.checked_mul(p)
                    }
                }
            };
            if let Some(n) = normalized {
                total_units = total_units.saturating_add(n);
            }
        }
    }
    Ok(format_scaled(total_units, SCALE))
}

async fn compute_current_streak(pool: &SqlitePool) -> Result<i64, String> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT date(created_at, 'unixepoch') AS d
        FROM token_usage_log
        ORDER BY d DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let today = chrono::Utc::now().date_naive();
    let mut streak = 0i64;
    let mut expected = today;
    for d in rows {
        let Some(date) = chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok() else {
            break;
        };
        if date != expected {
            break;
        }
        streak += 1;
        match expected.pred_opt() {
            Some(prev) => expected = prev,
            None => break,
        }
    }
    Ok(streak)
}

async fn fetch_heatmap(pool: &SqlitePool) -> Result<Vec<HeatmapPoint>, String> {
    let rows = sqlx::query(
        r#"
        SELECT
            date(created_at, 'unixepoch') AS d,
            COALESCE(SUM(total_tokens), 0) AS tokens,
            COUNT(DISTINCT conversation_id) AS sessions
        FROM token_usage_log
        WHERE conversation_id IS NOT NULL
        GROUP BY d
        ORDER BY d ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| HeatmapPoint {
            date: row.get("d"),
            tokens: row.get("tokens"),
            sessions: row.get("sessions"),
        })
        .collect())
}

async fn fetch_model_breakdown(pool: &SqlitePool) -> Result<Vec<ModelBreakdown>, String> {
    let rows = sqlx::query(
        r#"
        SELECT
            model,
            COALESCE(SUM(total_tokens), 0) AS tokens,
            COUNT(*) AS calls
        FROM token_usage_log
        GROUP BY model
        ORDER BY tokens DESC, model ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let model: String = row.get("model");
        let tokens: i64 = row.get("tokens");
        let calls: i64 = row.get("calls");
        let cost_usd = sum_cost_rows(
            pool,
            "SELECT cost_usd FROM token_usage_log WHERE model = ? AND cost_usd IS NOT NULL AND cost_usd != ''",
            &[&model],
        )
        .await?;
        result.push(ModelBreakdown {
            model,
            tokens,
            calls,
            cost_usd,
        });
    }
    Ok(result)
}
