//! Stable session-usage decision values shared by orchestration and API mapping.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionUsageLimitBasis {
    ContextWindow,
    PromptThreshold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionUsage {
    pub measured_prompt_tokens: Option<u64>,
    pub current_tokens: u64,
    pub projected_tokens: Option<u64>,
    pub limit_tokens: Option<u64>,
    pub limit_basis: Option<SessionUsageLimitBasis>,
    pub reserved_tokens: Option<u32>,
    pub model_context_window_tokens: Option<u32>,
    pub model_max_input_tokens: Option<u32>,
    pub model_max_output_tokens: Option<u32>,
}
