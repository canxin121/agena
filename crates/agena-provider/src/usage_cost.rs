use std::time::{SystemTime, UNIX_EPOCH};

/// Price contribution normalized from provider-reported or estimated usage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompletionUsageCostContribution {
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_requests: u64,
}

impl CompletionUsageCostContribution {
    fn add_assign(&mut self, additional: Self) {
        self.total_cost_usd += additional.total_cost_usd;
        self.recorded_cost_usd += additional.recorded_cost_usd;
        self.estimated_cost_usd += additional.estimated_cost_usd;
        self.unpriced_requests = self
            .unpriced_requests
            .saturating_add(additional.unpriced_requests);
    }
}

/// Price only the usage owned by one provider/model observation.
pub fn completion_usage_own_cost_contribution(
    provider_id: &str,
    model_id: &str,
    usage: &crate::CompletionUsage,
) -> CompletionUsageCostContribution {
    let recorded = usage
        .recorded_cost_available
        .then_some(usage.recorded_cost.max(0.0));
    if let Some(recorded_cost_usd) = recorded {
        return CompletionUsageCostContribution {
            total_cost_usd: recorded_cost_usd,
            recorded_cost_usd,
            estimated_cost_usd: 0.0,
            unpriced_requests: u64::from(usage.cost_estimate_incomplete),
        };
    }

    let priced_units = usage
        .billable_items
        .iter()
        .filter_map(|item| item.cost_usd)
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
        .sum::<f64>();
    let token_estimate = estimate_completion_usage_cost_usd(provider_id, model_id, usage);
    let estimated = if usage.estimated_cost.is_finite() && usage.estimated_cost > 0.0 {
        Some(usage.estimated_cost)
    } else {
        token_estimate
            .map(|value| value + priced_units)
            .or_else(|| (priced_units > 0.0).then_some(priced_units))
    };
    let incomplete = usage.cost_estimate_incomplete
        || usage
            .billable_items
            .iter()
            .any(|item| item.cost_usd.is_none())
        || (usage.has_own_usage() && estimated.is_none());
    CompletionUsageCostContribution {
        total_cost_usd: estimated.unwrap_or_default(),
        recorded_cost_usd: 0.0,
        estimated_cost_usd: estimated.unwrap_or_default(),
        unpriced_requests: u64::from(incomplete),
    }
}

/// Price an outer completion and every attributed nested provider-tool request.
pub fn completion_usage_cost_contribution(
    provider_id: &str,
    model_id: &str,
    usage: &crate::CompletionUsage,
) -> CompletionUsageCostContribution {
    let mut result = completion_usage_own_cost_contribution(provider_id, model_id, usage);
    for attributed in &usage.attributed_usage {
        result.add_assign(completion_usage_cost_contribution(
            attributed.provider_id.as_str(),
            attributed.model_id.as_str(),
            attributed.usage.as_ref(),
        ));
    }
    result
}

/// Estimate token charges from Agena's dated built-in pricing snapshot.
///
/// Provider-reported cost and configured model-catalog pricing remain preferred.
/// Unknown models or unrepresented account modifiers are never treated as free.
pub fn estimate_completion_usage_cost_usd(
    provider_id: &str,
    model_id: &str,
    usage: &crate::CompletionUsage,
) -> Option<f64> {
    let rates = estimate_model_token_rates(provider_id, model_id)?;
    let long_context = rates
        .long_context_threshold
        .is_some_and(|threshold| usage.own_cache_input_tokens() > threshold);
    let input_multiplier = if long_context {
        rates.long_input_multiplier
    } else {
        1.0
    };
    let output_multiplier = if long_context {
        rates.long_output_multiplier
    } else {
        1.0
    };
    let output_tokens = usage.output_tokens.saturating_add(usage.reasoning_tokens) as f64;
    let ttl_writes = usage
        .cache_write_5m_tokens
        .saturating_add(usage.cache_write_1h_tokens);
    let generic_writes = usage.cache_write_tokens.saturating_sub(ttl_writes);
    let provider = provider_id.trim().to_ascii_lowercase();
    let anthropic = provider.contains("anthropic") || provider.contains("claude");
    let write_5m_rate = if anthropic {
        rates.input_per_million * 1.25
    } else {
        rates.cache_write_per_million
    };
    let write_1h_rate = if anthropic {
        rates.input_per_million * 2.0
    } else {
        rates.cache_write_per_million
    };
    let cost = (usage.input_tokens as f64 * rates.input_per_million * input_multiplier
        + output_tokens * rates.output_per_million * output_multiplier
        + generic_writes as f64 * rates.cache_write_per_million * input_multiplier
        + usage.cache_write_5m_tokens as f64 * write_5m_rate * input_multiplier
        + usage.cache_write_1h_tokens as f64 * write_1h_rate * input_multiplier
        + usage.cache_read_tokens as f64 * rates.cache_read_per_million * input_multiplier)
        / PER_MILLION;
    cost.is_finite().then_some(cost.max(0.0))
}

const PER_MILLION: f64 = 1_000_000.0;
const SEPTEMBER_1_2026_UTC: u64 = 1_788_220_800;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ModelTokenRates {
    input_per_million: f64,
    output_per_million: f64,
    cache_write_per_million: f64,
    cache_read_per_million: f64,
    long_context_threshold: Option<u64>,
    long_input_multiplier: f64,
    long_output_multiplier: f64,
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
            long_context_threshold: None,
            long_input_multiplier: 1.0,
            long_output_multiplier: 1.0,
        }
    }

    const fn tiered(
        mut self,
        threshold: u64,
        input_multiplier: f64,
        output_multiplier: f64,
    ) -> Self {
        self.long_context_threshold = Some(threshold);
        self.long_input_multiplier = input_multiplier;
        self.long_output_multiplier = output_multiplier;
        self
    }
}

fn estimate_model_token_rates(provider_id: &str, model_id: &str) -> Option<ModelTokenRates> {
    let provider = provider_id.trim().to_ascii_lowercase();
    let model = normalize_model_id(model_id);

    if looks_like_local_model(model.as_str()) || provider == "ollama" {
        return Some(ModelTokenRates::new(0.0, 0.0, 0.0, 0.0));
    }

    // Claude Sonnet 5 has a published promotional rate through 2026-08-31.
    if model.starts_with("claude-sonnet-5") {
        let after_promotion = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .is_some_and(|duration| duration.as_secs() >= SEPTEMBER_1_2026_UTC);
        return Some(if after_promotion {
            ModelTokenRates::new(3.0, 15.0, 3.75, 0.30)
        } else {
            ModelTokenRates::new(2.0, 10.0, 2.50, 0.20)
        });
    }

    for (prefix, rates) in PRICING_PREFIXES {
        if model == *prefix || model.starts_with(&format!("{prefix}-")) {
            return Some(*rates);
        }
    }

    if provider.contains("anthropic") || provider.contains("claude") {
        if model.contains("fable") || model.contains("mythos") {
            return Some(ModelTokenRates::new(10.0, 50.0, 12.50, 1.0));
        }
        if model.contains("opus") {
            return Some(ModelTokenRates::new(5.0, 25.0, 6.25, 0.50));
        }
        if model.contains("sonnet") {
            return Some(ModelTokenRates::new(3.0, 15.0, 3.75, 0.30));
        }
        if model.contains("haiku") {
            return Some(ModelTokenRates::new(1.0, 5.0, 1.25, 0.10));
        }
    }

    if provider.contains("openai") || provider.contains("chatgpt") {
        if model.contains("gpt-5.6-sol") {
            return Some(ModelTokenRates::new(5.0, 30.0, 6.25, 0.50).tiered(272_000, 2.0, 1.5));
        }
        if model.contains("gpt-5.6-terra") {
            return Some(ModelTokenRates::new(2.5, 15.0, 3.125, 0.25).tiered(272_000, 2.0, 1.5));
        }
        if model.contains("gpt-5.6-luna") {
            return Some(ModelTokenRates::new(1.0, 6.0, 1.25, 0.10).tiered(272_000, 2.0, 1.5));
        }
        if model.contains("gpt-5") {
            return Some(ModelTokenRates::new(1.25, 10.0, 1.25, 0.125));
        }
    }

    if provider.contains("gemini") || provider.contains("google") {
        if model.contains("3.6-flash") {
            return Some(ModelTokenRates::new(1.50, 7.50, 0.15, 0.15));
        }
        if model.contains("3.5-flash-lite") {
            return Some(ModelTokenRates::new(0.30, 2.50, 0.03, 0.03));
        }
        if model.contains("pro") {
            return Some(ModelTokenRates::new(2.0, 12.0, 0.20, 0.20).tiered(200_000, 2.0, 1.5));
        }
        if model.contains("flash") {
            return Some(ModelTokenRates::new(0.30, 2.50, 0.03, 0.03));
        }
    }

    None
}

const PRICING_PREFIXES: &[(&str, ModelTokenRates)] = &[
    (
        "claude-fable-5",
        ModelTokenRates::new(10.0, 50.0, 12.50, 1.0),
    ),
    (
        "claude-mythos-5",
        ModelTokenRates::new(10.0, 50.0, 12.50, 1.0),
    ),
    ("claude-opus-5", ModelTokenRates::new(5.0, 25.0, 6.25, 0.50)),
    (
        "claude-opus-4-8",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
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
    (
        "gpt-5.6-sol",
        ModelTokenRates::new(5.0, 30.0, 6.25, 0.50).tiered(272_000, 2.0, 1.5),
    ),
    (
        "gpt-5.6-terra",
        ModelTokenRates::new(2.5, 15.0, 3.125, 0.25).tiered(272_000, 2.0, 1.5),
    ),
    (
        "gpt-5.6-luna",
        ModelTokenRates::new(1.0, 6.0, 1.25, 0.10).tiered(272_000, 2.0, 1.5),
    ),
    (
        "gpt-5.5-pro",
        ModelTokenRates::new(30.0, 180.0, 30.0, 3.0).tiered(272_000, 2.0, 1.5),
    ),
    (
        "gpt-5.5",
        ModelTokenRates::new(5.0, 30.0, 5.0, 0.50).tiered(272_000, 2.0, 1.5),
    ),
    ("gpt-5.4-pro", ModelTokenRates::new(30.0, 180.0, 30.0, 3.0)),
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
        "gemini-3.6-flash",
        ModelTokenRates::new(1.50, 7.50, 0.15, 0.15),
    ),
    (
        "gemini-3.5-flash-lite",
        ModelTokenRates::new(0.30, 2.50, 0.03, 0.03),
    ),
    (
        "gemini-3.1-pro-preview",
        ModelTokenRates::new(2.0, 12.0, 0.20, 0.20).tiered(200_000, 2.0, 1.5),
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
        ModelTokenRates::new(1.25, 10.0, 0.125, 0.125).tiered(200_000, 2.0, 1.5),
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
        "claude-opus-4.8" | "claude-4.8-opus" => "claude-opus-4-8".to_string(),
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
