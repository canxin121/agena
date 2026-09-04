use serde::{Deserialize, Serialize};

use crate::UsageBillableUnitTotal;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Cost and token breakdown per model.
pub struct ModelCostBreakdown {
    pub provider_id: String,
    pub model_id: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_write_5m_tokens: u64,
    pub cache_write_1h_tokens: u64,
    pub cache_read_tokens: u64,
    pub tool_use_tokens: u64,
    pub other_tokens: u64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_requests: u64,
    pub billable_units: Vec<UsageBillableUnitTotal>,
}

impl ModelCostBreakdown {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.other_tokens)
    }

    pub fn cache_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn cache_hit_rate(&self) -> f64 {
        ratio(self.cache_read_tokens, self.cache_input_tokens())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Aggregated cost summary for a session.
pub struct SessionCostSummary {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_write_5m_tokens: u64,
    pub cache_write_1h_tokens: u64,
    pub cache_read_tokens: u64,
    pub tool_use_tokens: u64,
    pub other_tokens: u64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_requests: u64,
    pub billable_units: Vec<UsageBillableUnitTotal>,
    pub by_model: Vec<ModelCostBreakdown>,
}

impl SessionCostSummary {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.other_tokens)
    }

    pub fn cache_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn cache_hit_rate(&self) -> f64 {
        ratio(self.cache_read_tokens, self.cache_input_tokens())
    }

    pub fn is_empty(&self) -> bool {
        self.requests == 0
    }

    pub fn one_line(&self) -> String {
        if self.is_empty() {
            return "no usage recorded yet".to_string();
        }
        let cache_tokens = self
            .cache_write_tokens
            .saturating_add(self.cache_read_tokens);
        format!(
            "{} in{} + {} out{} = {} tokens{} · ${:.4} over {} request{}",
            format_count(self.input_tokens),
            if cache_tokens > 0 {
                format!(" + {} cache", format_count(cache_tokens))
            } else {
                String::new()
            },
            format_count(self.output_tokens),
            if self.reasoning_tokens > 0 {
                format!(" + {} reasoning", format_count(self.reasoning_tokens))
            } else {
                String::new()
            },
            format_count(self.total_tokens()),
            if self.cache_input_tokens() > 0 {
                format!(" · {:.1}% cache hit", self.cache_hit_rate() * 100.0)
            } else {
                String::new()
            },
            self.total_cost_usd,
            self.requests,
            if self.requests == 1 { "" } else { "s" },
        )
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::SessionCostSummary;

    #[test]
    fn one_line_formats_summary_without_core_message_types() {
        let summary = SessionCostSummary {
            requests: 2,
            input_tokens: 1_200,
            output_tokens: 34,
            cache_read_tokens: 50,
            total_cost_usd: 0.1234,
            ..Default::default()
        };

        assert_eq!(
            summary.one_line(),
            "1,200 in + 50 cache + 34 out = 1,284 tokens · 4.0% cache hit · $0.1234 over 2 requests"
        );
    }
}
