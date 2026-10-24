/// Runtime policy for deciding whether an already-measured prompt payload
/// should be compacted before provider execution.
#[derive(Debug, Clone)]
/// Governor enforcing context window limits.
pub struct ContextGovernor {
    policy: agena_domain::ContextPolicy,
}

impl ContextGovernor {
    pub fn new(policy: agena_domain::ContextPolicy) -> Self {
        Self { policy }
    }

    pub fn prompt_exceeds_budget(&self, payload_chars: usize, max_prompt_chars: usize) -> bool {
        let proactive = self.policy.proactive_char_threshold(max_prompt_chars);
        payload_chars > max_prompt_chars || payload_chars > proactive
    }

    pub fn max_prompt_chars(&self) -> usize {
        self.policy.max_prompt_chars
    }
}
