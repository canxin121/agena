use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, FromJsonQueryResult)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

impl MessageUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn saturating_sub(&self, earlier: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(earlier.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(earlier.output_tokens),
            reasoning_tokens: self
                .reasoning_tokens
                .saturating_sub(earlier.reasoning_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_sub(earlier.cache_write_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_sub(earlier.cache_read_tokens),
            total_cost: (self.total_cost - earlier.total_cost).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MessageUsage;

    #[test]
    fn usage_delta_is_scoped_to_the_current_invocation() {
        let before = MessageUsage {
            input_tokens: 100,
            output_tokens: 20,
            total_cost: 0.25,
            ..Default::default()
        };
        let after = MessageUsage {
            input_tokens: 145,
            output_tokens: 32,
            reasoning_tokens: 7,
            total_cost: 0.40,
            ..Default::default()
        };

        let delta = after.saturating_sub(&before);
        assert_eq!(delta.input_tokens, 45);
        assert_eq!(delta.output_tokens, 12);
        assert_eq!(delta.reasoning_tokens, 7);
        assert!((delta.total_cost - 0.15).abs() < 1e-12);
    }
}
