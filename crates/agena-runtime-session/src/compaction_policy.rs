//! Version of the runtime's bounded kernel compaction strategy.
/// Failure suppression belongs to a strategy, so repaired strategies can
/// recover sessions disabled by an older implementation.
pub const COMPACTION_POLICY_VERSION: u64 = 7;
