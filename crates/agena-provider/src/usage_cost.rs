/// Price contribution normalized from provider-reported or estimated usage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompletionUsageCostContribution {
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_runs: u64,
}

/// Prefer a provider-reported charge, otherwise estimate from the built-in
/// pricing table and explicitly report an unpriced usage observation.
pub fn completion_usage_cost_contribution(
    provider_id: &str,
    model_id: &str,
    usage: &crate::CompletionUsage,
) -> CompletionUsageCostContribution {
    if usage.total_cost.is_finite() && usage.total_cost > 0.0 {
        return CompletionUsageCostContribution {
            total_cost_usd: usage.total_cost,
            recorded_cost_usd: usage.total_cost,
            estimated_cost_usd: 0.0,
            unpriced_runs: 0,
        };
    }
    if let Some(estimated_cost_usd) =
        estimate_completion_usage_cost_usd(provider_id, model_id, usage)
    {
        return CompletionUsageCostContribution {
            total_cost_usd: estimated_cost_usd,
            recorded_cost_usd: 0.0,
            estimated_cost_usd,
            unpriced_runs: 0,
        };
    }
    CompletionUsageCostContribution {
        total_cost_usd: 0.0,
        recorded_cost_usd: 0.0,
        estimated_cost_usd: 0.0,
        unpriced_runs: 1,
    }
}

/// Estimate cost from a compact built-in pricing table when providers do not
/// return a charge. The rates are intentionally conservative and family-based;
/// exact provider billing should still be preferred when [`crate::CompletionUsage`]
/// carries `total_cost`.
pub fn estimate_completion_usage_cost_usd(
    provider_id: &str,
    model_id: &str,
    usage: &crate::CompletionUsage,
) -> Option<f64> {
    let rates = estimate_model_token_rates(provider_id, model_id)?;
    let output_tokens = usage.output_tokens.saturating_add(usage.reasoning_tokens) as f64;
    let cost = (usage.input_tokens as f64 * rates.input_per_million
        + output_tokens * rates.output_per_million
        + usage.cache_write_tokens as f64 * rates.cache_write_per_million
        + usage.cache_read_tokens as f64 * rates.cache_read_per_million)
        / PER_MILLION;
    cost.is_finite().then_some(cost.max(0.0))
}

const PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ModelTokenRates {
    input_per_million: f64,
    output_per_million: f64,
    cache_write_per_million: f64,
    cache_read_per_million: f64,
}

impl ModelTokenRates {
    const fn new(
        input_per_million: f64,
        output_per_million: f64,
        cache_write_per_million: f64,
        cache_read_per_million: f64,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            cache_write_per_million,
            cache_read_per_million,
        }
    }
}

fn estimate_model_token_rates(provider_id: &str, model_id: &str) -> Option<ModelTokenRates> {
    let provider = provider_id.trim().to_ascii_lowercase();
    let model = normalize_model_id(model_id);

    if looks_like_local_model(model.as_str()) || provider == "ollama" {
        return Some(ModelTokenRates::new(0.0, 0.0, 0.0, 0.0));
    }

    for (prefix, rates) in PRICING_PREFIXES {
        if model == *prefix || model.starts_with(&format!("{prefix}-")) {
            return Some(*rates);
        }
    }

    if provider.contains("anthropic") {
        if model.contains("opus") {
            return Some(ModelTokenRates::new(15.0, 75.0, 18.75, 1.50));
        }
        if model.contains("sonnet") {
            return Some(ModelTokenRates::new(3.0, 15.0, 3.75, 0.30));
        }
        if model.contains("haiku") {
            return Some(ModelTokenRates::new(0.80, 4.0, 1.00, 0.08));
        }
    }

    if provider.contains("openai") && model.contains("gpt-5") {
        return Some(ModelTokenRates::new(1.25, 10.0, 1.25, 0.125));
    }

    if provider.contains("gemini") || provider.contains("google") {
        if model.contains("pro") {
            return Some(ModelTokenRates::new(1.25, 10.0, 1.25, 0.125));
        }
        if model.contains("flash") {
            return Some(ModelTokenRates::new(0.30, 2.50, 0.30, 0.03));
        }
    }

    None
}

const PRICING_PREFIXES: &[(&str, ModelTokenRates)] = &[
    (
        "claude-opus-4-7",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
    (
        "claude-opus-4-6",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
    (
        "claude-opus-4-5",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
    (
        "claude-opus-4",
        ModelTokenRates::new(15.0, 75.0, 18.75, 1.50),
    ),
    (
        "claude-sonnet-4",
        ModelTokenRates::new(3.0, 15.0, 3.75, 0.30),
    ),
    (
        "claude-haiku-4-5",
        ModelTokenRates::new(1.0, 5.0, 1.25, 0.10),
    ),
    (
        "claude-3-5-haiku",
        ModelTokenRates::new(0.80, 4.0, 1.00, 0.08),
    ),
    (
        "claude-haiku-3-5",
        ModelTokenRates::new(0.80, 4.0, 1.00, 0.08),
    ),
    (
        "claude-haiku-3",
        ModelTokenRates::new(0.25, 1.25, 0.30, 0.03),
    ),
    ("gpt-5.5-pro", ModelTokenRates::new(30.0, 180.0, 30.0, 30.0)),
    ("gpt-5.5", ModelTokenRates::new(5.0, 30.0, 5.0, 0.50)),
    ("gpt-5.4-pro", ModelTokenRates::new(30.0, 180.0, 30.0, 30.0)),
    (
        "gpt-5.4-mini",
        ModelTokenRates::new(0.75, 4.50, 0.75, 0.075),
    ),
    ("gpt-5.4-nano", ModelTokenRates::new(0.20, 1.25, 0.20, 0.02)),
    ("gpt-5.4", ModelTokenRates::new(2.50, 15.0, 2.50, 0.25)),
    (
        "gpt-5.3-codex",
        ModelTokenRates::new(1.75, 14.0, 1.75, 0.175),
    ),
    (
        "gpt-5.2-codex",
        ModelTokenRates::new(1.75, 14.0, 1.75, 0.175),
    ),
    ("gpt-5.2", ModelTokenRates::new(1.75, 14.0, 1.75, 0.175)),
    ("gpt-5-nano", ModelTokenRates::new(0.05, 0.40, 0.05, 0.005)),
    ("gpt-5-mini", ModelTokenRates::new(0.25, 2.0, 0.25, 0.025)),
    ("gpt-5", ModelTokenRates::new(1.25, 10.0, 1.25, 0.125)),
    (
        "gpt-4.1-nano",
        ModelTokenRates::new(0.10, 0.40, 0.10, 0.025),
    ),
    ("gpt-4.1-mini", ModelTokenRates::new(0.40, 1.60, 0.40, 0.10)),
    ("gpt-4.1", ModelTokenRates::new(2.0, 8.0, 2.0, 0.50)),
    ("gpt-4o-mini", ModelTokenRates::new(0.15, 0.60, 0.15, 0.075)),
    ("gpt-4o", ModelTokenRates::new(2.50, 10.0, 2.50, 1.25)),
    (
        "gemini-3.1-pro-preview",
        ModelTokenRates::new(2.0, 12.0, 0.20, 0.20),
    ),
    (
        "gemini-3.1-flash-lite",
        ModelTokenRates::new(0.25, 1.50, 0.025, 0.025),
    ),
    (
        "gemini-3-flash-preview",
        ModelTokenRates::new(0.50, 3.0, 0.05, 0.05),
    ),
    (
        "gemini-2.5-pro",
        ModelTokenRates::new(1.25, 10.0, 0.125, 0.125),
    ),
    (
        "gemini-2.5-flash",
        ModelTokenRates::new(0.30, 2.50, 0.03, 0.03),
    ),
    (
        "gemini-2.5-flash-lite",
        ModelTokenRates::new(0.10, 0.40, 0.01, 0.01),
    ),
    (
        "gemini-2.0-flash",
        ModelTokenRates::new(0.10, 0.40, 0.025, 0.025),
    ),
];

fn normalize_model_id(model_id: &str) -> String {
    let mut model = model_id.trim().to_ascii_lowercase();
    if let Some((_, stripped)) = model.rsplit_once('/') {
        model = stripped.to_string();
    }
    if let Some((stripped, _)) = model.split_once('@') {
        model = stripped.to_string();
    }
    if model.len() > 9
        && model.get(model.len().saturating_sub(9)..model.len().saturating_sub(8)) == Some("-")
        && model[model.len().saturating_sub(8)..]
            .chars()
            .all(|ch| ch.is_ascii_digit())
    {
        model.truncate(model.len() - 9);
    }

    match model.as_str() {
        "claude-sonnet-4.6" | "claude-4.6-sonnet" | "claude-4.6-sonnet-thinking" => {
            "claude-sonnet-4-6".to_string()
        }
        "claude-sonnet-4.5" | "claude-4.5-sonnet" | "claude-4.5-sonnet-thinking" => {
            "claude-sonnet-4-5".to_string()
        }
        "claude-opus-4.7" | "claude-4.7-opus" => "claude-opus-4-7".to_string(),
        "claude-opus-4.6" | "claude-4.6-opus" => "claude-opus-4-6".to_string(),
        "claude-opus-4.5" | "claude-4.5-opus" => "claude-opus-4-5".to_string(),
        "claude-haiku-4.5" | "claude-4.5-haiku" => "claude-haiku-4-5".to_string(),
        "cursor-auto" | "cursor-agent-auto" | "kiro-auto" | "cline-auto" => {
            "claude-sonnet-4-5".to_string()
        }
        "gpt-5.1-codex" => "gpt-5".to_string(),
        _ => model,
    }
}

fn looks_like_local_model(model: &str) -> bool {
    model.contains(':')
        || model.ends_with(".gguf")
        || model.contains("-gguf")
        || model.contains("-q4")
        || model.contains("-q5")
        || model.contains("-q6")
        || model.contains("-q8")
        || model.contains("llama")
}
