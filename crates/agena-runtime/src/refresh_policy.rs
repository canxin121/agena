//! Provider-neutral startup refresh decision policy.

use chrono::{DateTime, Duration, Utc};

/// Decide whether a cached catalog should be refreshed on startup.
pub fn should_refresh(
    has_entries: bool,
    last_refresh_at: Option<DateTime<Utc>>,
    max_age: Duration,
) -> bool {
    !has_entries || crate::is_stale(last_refresh_at, max_age)
}
