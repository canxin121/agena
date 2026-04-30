use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicy {
    pub max_messages: usize,
    pub max_prompt_chars: usize,
    pub keep_tail_messages: usize,
    pub max_compaction_rounds: u8,
    /// Proactive compaction trigger. When set, compaction fires when the
    /// projected prompt size has consumed more than `100 - headroom_pct`
    /// percent of the budget — *before* the model returns a context-overflow
    /// error. `0` disables the proactive trigger and falls back to the
    /// historic "exceeded the budget" behavior.
    pub compaction_headroom_pct: u8,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            max_messages: 24,
            max_prompt_chars: 96_000,
            keep_tail_messages: 12,
            max_compaction_rounds: 1,
            compaction_headroom_pct: 15,
        }
    }
}

impl ContextPolicy {
    /// `max_prompt_chars * (100 - headroom_pct) / 100`. Returns
    /// `max_prompt_chars` when the headroom is disabled.
    pub fn proactive_char_threshold(&self, max_prompt_chars: usize) -> usize {
        if self.compaction_headroom_pct == 0 || self.compaction_headroom_pct >= 100 {
            return max_prompt_chars;
        }
        let factor = 100u64.saturating_sub(self.compaction_headroom_pct as u64);
        max_prompt_chars
            .saturating_mul(factor as usize)
            / 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proactive_threshold_default_is_85_percent_of_budget() {
        let p = ContextPolicy::default();
        assert_eq!(p.proactive_char_threshold(100_000), 85_000);
    }

    #[test]
    fn proactive_threshold_disabled_returns_full_budget() {
        let mut p = ContextPolicy::default();
        p.compaction_headroom_pct = 0;
        assert_eq!(p.proactive_char_threshold(100_000), 100_000);
    }

    #[test]
    fn proactive_threshold_clamps_pathological_pct() {
        let mut p = ContextPolicy::default();
        p.compaction_headroom_pct = 200;
        // >= 100 disables the threshold.
        assert_eq!(p.proactive_char_threshold(100_000), 100_000);
    }
}
