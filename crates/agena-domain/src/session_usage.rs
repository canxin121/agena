//! Stable session-usage decision values shared by orchestration and API mapping.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// What a session usage limit is based on.
pub enum SessionUsageLimitBasis {
    ContextWindow,
    PromptThreshold,
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Token usage state of a session.
pub struct SessionUsage {
    pub measured_prompt_tokens: Option<u64>,
    pub current_tokens: u64,
    pub projected_tokens: Option<u64>,
    /// Hard admission limit for the next request, after its output reservation.
    pub input_limit_tokens: Option<u64>,
    pub limit_tokens: Option<u64>,
    pub limit_basis: Option<SessionUsageLimitBasis>,
    pub reserved_tokens: Option<u32>,
    pub model_context_window_tokens: Option<u32>,
    pub model_max_input_tokens: Option<u32>,
    pub model_max_output_tokens: Option<u32>,
    /// Prompt tokens served from the provider cache by the last request.
    ///
    /// The server reports this so a client can show cache efficiency without
    /// reading the whole cost breakdown on every status refresh.
    pub last_cache_read_tokens: Option<u64>,
    /// Prompt tokens that could have been cached by the last request.
    pub last_cache_input_tokens: Option<u64>,
    /// `last_cache_read_tokens / last_cache_input_tokens` (0.0–1.0).
    pub last_cache_hit_rate: Option<f64>,
}
