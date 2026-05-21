use crate::message::Message;

use super::{ContextPolicy, prompt_window};

#[derive(Debug, Clone)]
pub struct ContextGovernor {
    policy: ContextPolicy,
}

impl ContextGovernor {
    pub fn new(policy: ContextPolicy) -> Self {
        Self { policy }
    }

    pub fn prompt_exceeds_budget(&self, messages: &[Message], max_prompt_chars: usize) -> bool {
        let chars = prompt_window::approximate_prompt_payload_chars(messages);
        let proactive = self.policy.proactive_char_threshold(max_prompt_chars);
        chars > max_prompt_chars || chars > proactive
    }

    pub fn max_prompt_chars(&self) -> usize {
        self.policy.max_prompt_chars
    }
}
