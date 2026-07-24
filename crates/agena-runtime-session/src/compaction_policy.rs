//! Runtime-owned bounds for transcript compaction.
//!
//! The compaction algorithm still adapts private Runtime message/session aggregates, but
//! its bounded-history and retry policy is independent of those aggregates.

/// Number of recent user turns retained in the compacted context suffix.
pub const MAX_RECENT_USER_TURNS: usize = 2;
/// Character budget for the recent context suffix sent to the compactor.
pub const MAX_RECENT_CONTEXT_CHARS: usize = 32_000;
/// Per-message character bound used while preparing compaction context.
pub const MAX_COMPACTOR_MESSAGE_CHARS: usize = 8_000;
/// Default output-token budget for the compaction request.
pub const DEFAULT_COMPACTION_OUTPUT_TOKENS: u32 = 4_096;
/// Number of failed compaction attempts before the session disables retries.
pub const MAX_COMPACTION_FAILURES: u8 = 3;
