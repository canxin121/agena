//! Prompt context policy values independent of session orchestration.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextPolicy {
    pub max_prompt_chars: usize,
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
    pub fn proactive_char_threshold(&self, max_prompt_chars: usize) -> usize {
        if self.prompt_budget_headroom_pct == 0 || self.prompt_budget_headroom_pct >= 100 {
            return max_prompt_chars;
        }
        let factor = 100u64.saturating_sub(self.prompt_budget_headroom_pct as u64);
        max_prompt_chars.saturating_mul(factor as usize) / 100
    }
}
