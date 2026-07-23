//! Generic time-based cache staleness policy.

use chrono::{DateTime, Duration, Utc};

/// Return whether a timestamp is absent or older than the supplied max age.
/// Future timestamps are treated as fresh to avoid refresh loops after clock
/// adjustments.
pub fn is_stale(last_refresh_at: Option<DateTime<Utc>>, max_age: Duration) -> bool {
    let Some(last_refresh_at) = last_refresh_at else {
        return true;
    };
    let age = Utc::now() - last_refresh_at;
    age >= Duration::zero() && age > max_age
}
