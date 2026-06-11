use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

static MODEL_DB_RAW: &str = include_str!("../../windowTokens/models.json");

#[derive(Debug, Clone, Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelEntry {
    id: String,
    pricing: ModelPricing,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelPricing {
    pub prompt: String,
    pub completion: String,
    #[serde(default)]
    pub input_cache_read: Option<String>,
    #[serde(default)]
    pub input_cache_write: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBilling {
    FreshInputOnly,
    InputIncludesCacheRead,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsageBreakdown {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_creation_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCostBreakdown {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    pub billable_input_tokens: u32,
    pub input_cost_usd: String,
    pub output_cost_usd: String,
    pub cache_read_cost_usd: String,
    pub cache_creation_cost_usd: String,
    pub total_cost_usd: String,
    pub pricing_model: String,
}

fn models() -> &'static Vec<ModelEntry> {
    static LIST: OnceLock<Vec<ModelEntry>> = OnceLock::new();
    LIST.get_or_init(|| {
        serde_json::from_str::<ModelList>(MODEL_DB_RAW)
            .map(|list| list.data)
            .unwrap_or_default()
    })
}

pub fn get_model_pricing(model: &str) -> Option<(String, ModelPricing)> {
    let key = model.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }

    models()
        .iter()
        .find(|entry| {
            let id = entry.id.trim().to_ascii_lowercase();
            id == key || id.rsplit('/').next().is_some_and(|slug| slug == key)
        })
        .map(|entry| (entry.id.clone(), entry.pricing.clone()))
}

pub fn cache_billing_for_provider(provider_name: &str) -> CacheBilling {
    match provider_name {
        "anthropic" => CacheBilling::FreshInputOnly,
        _ => CacheBilling::InputIncludesCacheRead,
    }
}

pub fn calculate_for_model(
    model: &str,
    usage: &TokenUsageBreakdown,
    cache_billing: CacheBilling,
) -> Option<TurnCostBreakdown> {
    let (pricing_model, pricing) = get_model_pricing(model)?;
    calculate_turn_cost(usage, &pricing, cache_billing).map(|mut cost| {
        cost.pricing_model = pricing_model;
        cost
    })
}

fn calculate_turn_cost(
    usage: &TokenUsageBreakdown,
    pricing: &ModelPricing,
    cache_billing: CacheBilling,
) -> Option<TurnCostBreakdown> {
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    let cache_read_tokens = usage.cache_read_tokens.unwrap_or(0);
    let cache_creation_tokens = usage.cache_creation_tokens.unwrap_or(0);
    let billable_input_tokens = match cache_billing {
        CacheBilling::FreshInputOnly => input_tokens,
        CacheBilling::InputIncludesCacheRead => input_tokens.saturating_sub(cache_read_tokens),
    };

    let input_cost_usd = multiply_decimal_by_u32(&pricing.prompt, billable_input_tokens)?;
    let output_cost_usd = multiply_decimal_by_u32(&pricing.completion, output_tokens)?;
    let cache_read_cost_usd = multiply_decimal_by_u32(
        pricing.input_cache_read.as_deref().unwrap_or("0"),
        cache_read_tokens,
    )?;
    let cache_creation_cost_usd = multiply_decimal_by_u32(
        pricing.input_cache_write.as_deref().unwrap_or("0"),
        cache_creation_tokens,
    )?;
    let total_cost_usd = sum_decimal_strings(&[
        &input_cost_usd,
        &output_cost_usd,
        &cache_read_cost_usd,
        &cache_creation_cost_usd,
    ])?;

    Some(TurnCostBreakdown {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        billable_input_tokens,
        input_cost_usd,
        output_cost_usd,
        cache_read_cost_usd,
        cache_creation_cost_usd,
        total_cost_usd,
        pricing_model: String::new(),
    })
}

fn multiply_decimal_by_u32(decimal: &str, multiplier: u32) -> Option<String> {
    let (units, scale) = parse_decimal(decimal)?;
    format_scaled(units.checked_mul(multiplier as u128)?, scale)
}

fn sum_decimal_strings(values: &[&str]) -> Option<String> {
    const SCALE: u32 = 18;
    let mut total = 0u128;
    for value in values {
        let (units, scale) = parse_decimal(value)?;
        let normalized = if scale > SCALE {
            units / pow10(scale - SCALE)?
        } else {
            units.checked_mul(pow10(SCALE - scale)?)?
        };
        total = total.checked_add(normalized)?;
    }
    format_scaled(total, SCALE)
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

fn format_scaled(units: u128, scale: u32) -> Option<String> {
    if scale == 0 {
        return Some(units.to_string());
    }
    let divisor = pow10(scale)?;
    let whole = units / divisor;
    let fraction = units % divisor;
    if fraction == 0 {
        return Some(whole.to_string());
    }
    let mut fraction_text = format!("{:0width$}", fraction, width = scale as usize);
    while fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    Some(format!("{whole}.{fraction_text}"))
}

fn pow10(power: u32) -> Option<u128> {
    let mut value = 1u128;
    for _ in 0..power {
        value = value.checked_mul(10)?;
    }
    Some(value)
}
