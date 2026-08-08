//! Stable cache statistics value; cache storage remains an orchestration detail.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Statistics for the session cache.
pub struct SessionCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub session_count: usize,
    pub total_bytes: usize,
}

/// Stable cache limits shared by session orchestration and presentation.
/// Cache storage and eviction remain runtime/session implementation details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCacheLimits {
    pub max_sessions: usize,
    pub ttl_secs: u64,
    pub max_bytes: usize,
}

impl Default for SessionCacheLimits {
    fn default() -> Self {
        Self {
            max_sessions: 128,
            ttl_secs: 15 * 60,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCacheLimits;

    #[test]
    fn default_limits_are_stable_and_nonzero() {
        let limits = SessionCacheLimits::default();
        assert_eq!(limits.max_sessions, 128);
        assert_eq!(limits.ttl_secs, 900);
        assert_eq!(limits.max_bytes, 64 * 1024 * 1024);
    }
}
