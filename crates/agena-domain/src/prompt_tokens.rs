//! Provider/session-neutral prompt token measurements.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
/// Token usage snapshot for a prompt or run.
pub struct PromptTokenUsageSnapshot {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
}

impl PromptTokenUsageSnapshot {
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens()
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
    }
}
