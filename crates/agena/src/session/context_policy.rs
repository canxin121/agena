use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicy {
    pub max_prompt_chars: usize,
    /// Prompt budget warning trigger. When set, the prompt is considered over
    /// budget once it has consumed more than `100 - headroom_pct` percent of
    /// the available budget. `0` disables the early warning and falls back to
    /// the full budget.
    pub prompt_budget_headroom_pct: u8,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            max_prompt_chars: 96_000,
            prompt_budget_headroom_pct: 15,
        }
    }
}

impl ContextPolicy {
    /// `max_prompt_chars * (100 - headroom_pct) / 100`. Returns
    /// `max_prompt_chars` when the headroom is disabled.
    pub fn proactive_char_threshold(&self, max_prompt_chars: usize) -> usize {
        if self.prompt_budget_headroom_pct == 0 || self.prompt_budget_headroom_pct >= 100 {
            return max_prompt_chars;
        }
        let factor = 100u64.saturating_sub(self.prompt_budget_headroom_pct as u64);
        max_prompt_chars.saturating_mul(factor as usize) / 100
    }
}
