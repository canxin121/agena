use crate::error::{AppError, ProviderErrorKind};
use crate::message::Message;
use crate::role::Role;

use super::ContextPolicy;

#[derive(Debug, Clone)]
pub struct ContextGovernor {
    policy: ContextPolicy,
}

impl ContextGovernor {
    pub fn new(policy: ContextPolicy) -> Self {
        Self { policy }
    }

    pub fn prepare_messages(&self, messages: &[Message]) -> Vec<Message> {
        if messages.len() <= self.policy.max_messages {
            return messages.to_vec();
        }

        let keep = self.policy.max_messages.min(messages.len());
        messages[messages.len() - keep..].to_vec()
    }

    pub fn compact_messages(&self, messages: &[Message]) -> Vec<Message> {
        if messages.is_empty() {
            return Vec::new();
        }

        let keep_tail = self.policy.keep_tail_messages.min(messages.len());
        let split = messages.len().saturating_sub(keep_tail);
        let head = &messages[..split];
        let tail = &messages[split..];

        let summary = head
            .iter()
            .map(|message| message.as_text_lossy())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let mut compacted = Vec::with_capacity(tail.len() + 1);
        compacted.push(Message::prompt_text(
            Role::System,
            format!("Context summary (compacted):\n{summary}"),
        ));
        compacted.extend_from_slice(tail);
        compacted
    }

    pub fn should_retry_with_compaction(&self, err: &AppError, rounds: u8) -> bool {
        rounds < self.policy.max_compaction_rounds
            && matches!(
                err.provider_error_kind(),
                Some(ProviderErrorKind::ContextOverflow)
            )
    }
}
