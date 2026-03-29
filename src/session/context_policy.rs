use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicy {
    pub max_messages: usize,
    pub keep_tail_messages: usize,
    pub max_compaction_rounds: u8,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            max_messages: 24,
            keep_tail_messages: 12,
            max_compaction_rounds: 1,
        }
    }
}
