use std::time::Duration;

/// Runtime cache limits derived from the stable session-cache configuration.
#[derive(Debug, Clone, Copy)]
pub struct SessionCachePolicy {
    pub max_sessions: usize,
    pub ttl: Duration,
    pub max_bytes: usize,
}

impl SessionCachePolicy {
    pub fn from_limits(limits: agena_domain::SessionCacheLimits) -> Self {
        Self {
            max_sessions: limits.max_sessions,
            ttl: Duration::from_secs(limits.ttl_secs),
            max_bytes: limits.max_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCachePolicy;

    #[test]
    fn converts_stable_cache_limits_to_runtime_duration() {
        let policy = SessionCachePolicy::from_limits(agena_domain::SessionCacheLimits {
            max_sessions: 3,
            ttl_secs: 42,
            max_bytes: 99,
        });
        assert_eq!(policy.max_sessions, 3);
        assert_eq!(policy.ttl.as_secs(), 42);
        assert_eq!(policy.max_bytes, 99);
    }
}
