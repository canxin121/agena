//! Auto-compaction policy values independent of session orchestration.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAutoCompactionConfig {
    pub enabled: bool,
    pub reserved_tokens: Option<u32>,
}

impl Default for SessionAutoCompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reserved_tokens: None,
        }
    }
}
