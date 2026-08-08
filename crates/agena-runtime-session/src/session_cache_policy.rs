//! Cache sizing for the v2 data facade.
//!
//! v1 kept a TTL/byte-bounded cache inside the session store. v2 replaced the
//! whole cache with the facade's [`MemoryLayer`](agena_storage::store::MemoryLayer),
//! whose only knob is `max_cached_sessions` (design 15.3). This policy bridges
//! the stable domain config ([`SessionCacheLimits`]) to that knob — the TTL and
//! byte cap of v1 have no v2 home and are deliberately dropped.

/// The facade cache size derived from the stable session-cache configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCachePolicy {
    pub max_sessions: usize,
}

impl SessionCachePolicy {
    pub fn from_limits(limits: agena_domain::SessionCacheLimits) -> Self {
        Self {
            max_sessions: limits.max_sessions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCachePolicy;

    #[test]
    fn converts_stable_cache_limits_to_the_facade_cache_size() {
        let policy = SessionCachePolicy::from_limits(agena_domain::SessionCacheLimits {
            max_sessions: 3,
            ttl_secs: 42,
            max_bytes: 99,
        });
        assert_eq!(policy.max_sessions, 3, "only the cache size reaches v2");
    }
}
